//! Exports people download from their networks, read as conversations for the local memory: a
//! WhatsApp chat (`WhatsApp Chat with Ana.txt`, or `_chat.txt` from a phone), a Telegram
//! Desktop export (`result.json`), Facebook's and Instagram's messages (`message_1.json`) and
//! LinkedIn's `messages.csv`. Each becomes lines like "2026-11-21 09:15 · Ana: Lunch at 1?"
//! under the conversation's name, so "what did Ana say about lunch" finds who said it and
//! when. They're read from the folders the person shared, like any file there: nothing leaves
//! the Mac, and an external brain sees them only as Privacy allows files.
use std::path::Path;

use serde_json::Value;

/// The largest export read.
pub const LARGEST: u64 = 32 * 1024 * 1024;
/// The most text kept from one export, its newest messages.
const KEPT: usize = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    WhatsApp,
    Telegram,
    Meta,
    LinkedIn,
}

/// What kind of export a file is, by its name.
pub fn kind(path: &Path) -> Option<Kind> {
    let name = path.file_name()?.to_str()?;
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".txt") && (lower.starts_with("whatsapp chat") || lower == "_chat.txt") {
        Some(Kind::WhatsApp)
    } else if lower == "result.json" {
        Some(Kind::Telegram)
    } else if lower.starts_with("message_") && lower.ends_with(".json") && lower[8..lower.len() - 5].chars().all(|c| c.is_ascii_digit()) {
        Some(Kind::Meta)
    } else if lower == "messages.csv" {
        Some(Kind::LinkedIn)
    } else {
        None
    }
}

/// The export as conversation lines, or None when it isn't the export its name says.
pub fn readable(kind: Kind, path: &Path, bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes));
    let conversations = match kind {
        Kind::WhatsApp => vec![whatsapp(&text, path)?],
        Kind::Telegram => telegram(&serde_json::from_str(&text).ok()?)?,
        Kind::Meta => vec![meta(&serde_json::from_str(&text).ok()?)?],
        Kind::LinkedIn => linkedin(&text)?,
    };
    let mut out = String::new();
    for (title, lines) in conversations.iter().filter(|(_, lines)| !lines.is_empty()) {
        out.push_str(&format!("Conversation: {title}\n"));
        for line in lines {
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
    }
    if out.is_empty() {
        return None;
    }
    // The newest messages are the ones kept.
    if out.len() > KEPT {
        let mut start = out.len() - KEPT;
        while !out.is_char_boundary(start) {
            start += 1;
        }
        out = out[start..].to_string();
    }
    Some(out)
}

fn line(when: &str, who: &str, said: &str) -> Option<String> {
    let said = said.split_whitespace().collect::<Vec<_>>().join(" ");
    let who = who.trim();
    (!said.is_empty() && !who.is_empty()).then(|| format!("{} · {who}: {said}", when.trim()))
}

/// "[21/11/2026, 09:15:02] Ana: text" from a phone, or "21/11/2026, 09:15 - Ana: text"; a line
/// that isn't a new message carries on the last one.
fn whatsapp(text: &str, path: &Path) -> Option<(String, Vec<String>)> {
    let title = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.strip_prefix("WhatsApp Chat with ").or_else(|| stem.strip_prefix("WhatsApp Chat - ")))
        .map(str::to_string)
        .or_else(|| path.parent()?.file_name()?.to_str().map(|folder| folder.trim_start_matches("WhatsApp Chat - ").to_string()))
        .unwrap_or_else(|| "WhatsApp".into());
    let mut messages: Vec<(String, String, String)> = Vec::new();
    for raw in text.lines() {
        let raw = raw.replace(['\u{200e}', '\u{200f}'], "");
        let raw = raw.trim_end();
        let started = |when: &str| when.len() >= 8 && when.len() <= 30 && when.chars().next().is_some_and(|c| c.is_ascii_digit()) && when.contains([':']) && when.contains(['/', '.', '-']);
        let found = if let Some(rest) = raw.strip_prefix('[') {
            rest.split_once("] ").filter(|(when, _)| started(when))
        } else {
            raw.split_once(" - ").filter(|(when, _)| started(when))
        };
        match found.and_then(|(when, rest)| Some((when, rest.split_once(": ")?))) {
            Some((when, (who, said))) if who.chars().count() <= 60 => messages.push((when.to_string(), who.to_string(), said.to_string())),
            // System lines ("Messages are end-to-end encrypted") have no speaker.
            _ if found.is_some() => {}
            _ => {
                if let Some(last) = messages.last_mut() {
                    last.2.push(' ');
                    last.2.push_str(raw);
                }
            }
        }
    }
    (!messages.is_empty()).then(|| (title, messages.iter().filter_map(|(when, who, said)| line(when, who, said)).collect()))
}

/// Telegram's text: a string, or pieces that are strings or `{ "text": … }`.
fn telegram_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts.iter().map(|part| part.as_str().map(str::to_string).or_else(|| part.get("text").and_then(Value::as_str).map(str::to_string)).unwrap_or_default()).collect(),
        _ => String::new(),
    }
}

fn telegram(export: &Value) -> Option<Vec<(String, Vec<String>)>> {
    let chats: Vec<&Value> = match export.pointer("/chats/list").and_then(Value::as_array) {
        Some(list) => list.iter().collect(),
        None => vec![export],
    };
    let conversations: Vec<(String, Vec<String>)> = chats
        .into_iter()
        .filter_map(|chat| {
            let messages = chat.get("messages")?.as_array()?;
            let title = chat.get("name").and_then(Value::as_str).unwrap_or("Telegram").to_string();
            let lines = messages
                .iter()
                .filter(|message| message.get("type").and_then(Value::as_str) == Some("message"))
                .filter_map(|message| {
                    let when = message.get("date").and_then(Value::as_str)?.replace('T', " ");
                    let when = when.get(..16).unwrap_or(&when).to_string();
                    line(&when, message.get("from").and_then(Value::as_str)?, &telegram_text(message.get("text")?))
                })
                .collect();
            Some((title, lines))
        })
        .collect();
    (!conversations.is_empty()).then_some(conversations)
}

/// Facebook and Instagram write UTF-8 as if each byte were a character: "â\u{80}\u{99}" for "’".
pub fn mended(text: &str) -> String {
    if text.chars().all(|c| (c as u32) < 256) {
        let bytes: Vec<u8> = text.chars().map(|c| c as u8).collect();
        if let Ok(fixed) = String::from_utf8(bytes) {
            return fixed;
        }
    }
    text.to_string()
}

fn meta(export: &Value) -> Option<(String, Vec<String>)> {
    let messages = export.get("messages")?.as_array()?;
    let title = export
        .get("title")
        .and_then(Value::as_str)
        .map(mended)
        .or_else(|| export.get("participants")?.as_array().map(|people| people.iter().filter_map(|person| person.get("name")?.as_str().map(mended)).collect::<Vec<_>>().join(", ")))
        .unwrap_or_else(|| "Messages".into());
    // Newest first in the file; oldest first here.
    let lines = messages
        .iter()
        .rev()
        .filter_map(|message| {
            let at = message.get("timestamp_ms").and_then(Value::as_i64)?;
            let when = chrono::DateTime::from_timestamp_millis(at)?.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string();
            line(&when, &mended(message.get("sender_name")?.as_str()?), &mended(message.get("content")?.as_str()?))
        })
        .collect();
    Some((title, lines))
}

/// Rows of a CSV file, with quoted fields that may hold commas, quotes and line breaks.
pub fn csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (true, c) => field.push(c),
            (false, '"') if field.is_empty() => quoted = true,
            (false, ',') => row.push(std::mem::take(&mut field)),
            (false, '\r') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            (false, c) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn linkedin(text: &str) -> Option<Vec<(String, Vec<String>)>> {
    let rows = csv(text);
    let (header, rows) = rows.split_first()?;
    let column = |name: &str| header.iter().position(|title| title.trim().eq_ignore_ascii_case(name));
    let (conversation, from, date, content) = (column("CONVERSATION ID")?, column("FROM")?, column("DATE")?, column("CONTENT")?);
    let title = column("CONVERSATION TITLE");
    let to = column("TO");
    let mut conversations: Vec<(String, String, Vec<(String, String)>)> = Vec::new();
    for row in rows {
        let field = |at: usize| row.get(at).map(String::as_str).unwrap_or_default();
        let id = field(conversation).to_string();
        // Without a title, both sides: "Ana Silva, Me".
        let name = title.map(field).filter(|title| !title.trim().is_empty()).map(str::to_string).unwrap_or_else(|| {
            [Some(field(from)), to.map(field)].into_iter().flatten().filter(|name| !name.trim().is_empty()).collect::<Vec<_>>().join(", ")
        });
        let Some(said) = line(&field(date).replace(" UTC", ""), field(from), field(content)) else { continue };
        match conversations.iter_mut().find(|(known, ..)| *known == id) {
            Some((_, _, lines)) => lines.push((field(date).to_string(), said)),
            None => conversations.push((id, name, vec![(field(date).to_string(), said)])),
        }
    }
    let out: Vec<(String, Vec<String>)> = conversations
        .into_iter()
        .map(|(_, name, mut lines)| {
            lines.sort_by(|a, b| a.0.cmp(&b.0));
            (name, lines.into_iter().map(|(_, line)| line).collect())
        })
        .collect();
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(kind: Kind, path: &str, text: &str) -> String {
        readable(kind, Path::new(path), text.as_bytes()).unwrap()
    }

    #[test]
    fn exports_are_known_by_their_names() {
        assert_eq!(kind(Path::new("/x/WhatsApp Chat with Ana.txt")), Some(Kind::WhatsApp));
        assert_eq!(kind(Path::new("/x/WhatsApp Chat - Ana/_chat.txt")), Some(Kind::WhatsApp));
        assert_eq!(kind(Path::new("/x/ChatExport/result.json")), Some(Kind::Telegram));
        assert_eq!(kind(Path::new("/x/inbox/ana_123/message_1.json")), Some(Kind::Meta));
        assert_eq!(kind(Path::new("/x/Basic_LinkedInDataExport/messages.csv")), Some(Kind::LinkedIn));
        for other in ["/x/notes.txt", "/x/message_one.json", "/x/results.json", "/x/report.csv"] {
            assert_eq!(kind(Path::new(other)), None, "{other}");
        }
    }

    #[test]
    fn a_whatsapp_chat_from_a_phone_or_a_computer_reads_alike() {
        let phone = "[21/11/2026, 09:15:02] Ana: Lunch at 1?\n\u{200e}[21/11/2026, 09:15:03] Messages and calls are end-to-end encrypted.\n[21/11/2026, 09:16:10] Me: Yes!\nSee you there\n";
        assert_eq!(read(Kind::WhatsApp, "/x/WhatsApp Chat - Ana/_chat.txt", phone), "Conversation: Ana\n21/11/2026, 09:15:02 · Ana: Lunch at 1?\n21/11/2026, 09:16:10 · Me: Yes! See you there\n\n");
        let computer = "11/21/26, 9:15 AM - Ana: Lunch at 1?\n11/21/26, 9:16 AM - Me: Yes\n";
        assert!(read(Kind::WhatsApp, "/x/WhatsApp Chat with Ana.txt", computer).contains("11/21/26, 9:15 AM · Ana: Lunch at 1?"));
        assert_eq!(readable(Kind::WhatsApp, Path::new("/x/_chat.txt"), b"just some notes"), None);
    }

    #[test]
    fn a_telegram_export_reads_each_chat_and_its_rich_text() {
        let export = r#"{"chats":{"list":[{"name":"Ana","messages":[
            {"type":"message","date":"2026-11-21T09:15:02","from":"Ana","text":"Lunch at 1?"},
            {"type":"service","date":"2026-11-21T09:15:30","actor":"Ana","action":"pin_message","text":""},
            {"type":"message","date":"2026-11-21T09:16:00","from":"Me","text":["See ",{"type":"link","text":"https://example.com"}," there"]}]}]}}"#;
        assert_eq!(read(Kind::Telegram, "/x/result.json", export), "Conversation: Ana\n2026-11-21 09:15 · Ana: Lunch at 1?\n2026-11-21 09:16 · Me: See https://example.com there\n\n");
        assert_eq!(readable(Kind::Telegram, Path::new("/x/result.json"), br#"{"results":[1,2]}"#), None);
    }

    #[test]
    fn facebook_and_instagram_messages_are_mended_and_put_in_order() {
        assert_eq!(mended("Itâ\u{80}\u{99}s here"), "It’s here");
        assert_eq!(mended("plain"), "plain");
        assert_eq!(mended("already ’ fine"), "already ’ fine");
        let export = r#"{"participants":[{"name":"Ana"},{"name":"Me"}],"messages":[
            {"sender_name":"Me","timestamp_ms":1795248960000,"content":"See you"},
            {"sender_name":"Ana","timestamp_ms":1795248900000,"content":"Lunch at 1? Itâ\u0080\u0099s on me"},
            {"sender_name":"Ana","timestamp_ms":1795248800000,"photos":[{"uri":"x.jpg"}]}],"title":"Ana"}"#;
        let text = read(Kind::Meta, "/x/inbox/ana/message_1.json", export);
        assert!(text.starts_with("Conversation: Ana\n"));
        let ana = text.find("Ana: Lunch at 1? It’s on me").unwrap();
        assert!(ana < text.find("Me: See you").unwrap(), "oldest first");
    }

    #[test]
    fn linkedin_messages_group_by_conversation_with_quoted_fields() {
        let csv_text = "CONVERSATION ID,CONVERSATION TITLE,FROM,SENDER PROFILE URL,TO,DATE,SUBJECT,CONTENT\r\n\
            c1,,Ana Silva,https://x,Me,2026-11-21 09:15:02 UTC,,\"Hi, are you free \"\"Friday\"\"?\nThanks\"\r\n\
            c1,,Me,https://y,Ana Silva,2026-11-21 10:00:00 UTC,,Yes\r\n";
        let text = read(Kind::LinkedIn, "/x/messages.csv", csv_text);
        assert_eq!(text, "Conversation: Ana Silva, Me\n2026-11-21 09:15:02 · Ana Silva: Hi, are you free \"Friday\"? Thanks\n2026-11-21 10:00:00 · Me: Yes\n\n");
        assert_eq!(csv("a,\"b,c\",d\n"), vec![vec!["a".to_string(), "b,c".into(), "d".into()]]);
        assert_eq!(readable(Kind::LinkedIn, Path::new("/x/messages.csv"), b"NAME,EMAIL\nAna,a@x\n"), None);
    }

    /// Through the reader shared folders go through, from files on disk: an export far past the
    /// usual limit for text reads as its conversation, and anything else as before.
    #[test]
    fn shared_folders_read_an_export_as_its_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let chat = dir.path().join("WhatsApp Chat with Ana.txt");
        let mut text = String::new();
        for n in 0..4_000 {
            text.push_str(&format!("[21/11/2026, 09:15:02] Ana: message {n} about the lease\n"));
        }
        assert!(text.len() as u64 > crate::local::MAX_READ);
        std::fs::write(&chat, &text).unwrap();
        let documents = crate::local::Documents::none();
        let read = crate::local::read_text(&mut std::fs::File::open(&chat).unwrap(), &chat, &documents).unwrap();
        assert!(read.starts_with("Conversation: Ana\n21/11/2026, 09:15:02 · Ana: message 0 about the lease"));
        // Read again, it comes from memory.
        assert_eq!(crate::local::read_text(&mut std::fs::File::open(&chat).unwrap(), &chat, &documents).unwrap(), read);
        // A file named like an export that isn't one reads as plain text.
        let other = dir.path().join("result.json");
        std::fs::write(&other, br#"{"score": 3}"#).unwrap();
        assert_eq!(crate::local::read_text(&mut std::fs::File::open(&other).unwrap(), &other, &documents).unwrap(), r#"{"score": 3}"#);
    }

    #[test]
    fn a_long_export_keeps_its_newest_messages() {
        let mut chat = String::new();
        for n in 0..20_000 {
            chat.push_str(&format!("[21/11/2026, 09:15:02] Ana: message number {n} with some words\n"));
        }
        let text = read(Kind::WhatsApp, "/x/WhatsApp Chat with Ana.txt", &chat);
        assert!(text.len() <= KEPT && text.contains("message number 19999") && !text.contains("message number 0 "));
    }
}
