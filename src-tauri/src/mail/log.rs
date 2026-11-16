//! Every change the hub makes in Mail, with what it takes to put it back. Mail gives an
//! email a new number when it moves, so an entry keeps the email's Message-ID and where the
//! change left it; Undo finds it there again. Drafts are never undone: Undo would have to
//! delete one, and the hub never deletes anything.
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::rules::Why;
use super::{Mailbox, Mark, Step, Target};

/// Entries kept on disk.
pub const KEEP: usize = 500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Change {
    /// Read, flagged or junk set on an email where it is.
    Mark { what: Mark, was: bool, now: bool, at: Mailbox, id: i64 },
    /// Moved; `id` is its number in `to`.
    Move { from: Mailbox, to: Mailbox, id: i64 },
    /// Marked junk and moved to the account's junk mailbox (or left where it was).
    Junk { from: Mailbox, to: Mailbox, was: bool, id: i64 },
    /// A draft saved in Drafts.
    Draft { id: i64, to: Vec<String>, attachments: Vec<String> },
    /// A reply saved as a draft.
    Reply { to: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    /// Unix seconds.
    pub at: i64,
    pub change: Change,
    /// The email's Message-ID (empty for a new draft).
    pub header: String,
    pub sender: String,
    pub subject: String,
    pub why: Why,
    pub undone: bool,
}

impl Entry {
    pub fn new(change: Change, target: Option<&Target>, subject: &str, why: Why) -> Self {
        Entry {
            id: uuid::Uuid::new_v4().to_string(),
            at: chrono::Utc::now().timestamp(),
            change,
            header: target.map(|t| t.header.clone()).unwrap_or_default(),
            sender: target.map(|t| t.sender.clone()).unwrap_or_default(),
            subject: subject.to_string(),
            why,
            undone: false,
        }
    }

    /// Whether Undo can put this back.
    pub fn undoable(&self) -> bool {
        !self.undone
            && !matches!(self.why, Why::Undo { .. })
            && matches!(self.change, Change::Mark { .. } | Change::Move { .. } | Change::Junk { .. })
    }

    /// One line: what changed, and on which email.
    pub fn label(&self) -> String {
        let what = match &self.change {
            Change::Mark { what: Mark::Read, now: true, .. } => "Read".to_string(),
            Change::Mark { what: Mark::Read, now: false, .. } => "Unread".to_string(),
            Change::Mark { what: Mark::Flagged, now: true, .. } => "Flagged".to_string(),
            Change::Mark { what: Mark::Flagged, now: false, .. } => "Unflagged".to_string(),
            Change::Mark { what: Mark::Junk, now: true, .. } => "Junk".to_string(),
            Change::Mark { what: Mark::Junk, now: false, .. } => "Not junk".to_string(),
            Change::Move { to, .. } => format!("To {}", leaf(&to.path)),
            Change::Junk { .. } => "Junk".to_string(),
            Change::Draft { to, attachments, .. } => match (to.first(), attachments.len()) {
                (Some(to), 0) => format!("Draft to {to}"),
                (Some(to), n) => format!("Draft to {to} · {n} 📎"),
                (None, n) if n > 0 => format!("Draft · {n} 📎"),
                (None, _) => "Draft".to_string(),
            },
            Change::Reply { to } => format!("Reply to {to}"),
        };
        if self.subject.is_empty() {
            what
        } else {
            format!("{what} · {}", self.subject)
        }
    }

    fn target(&self, mailbox: &Mailbox, id: i64) -> Target {
        Target {
            id,
            header: self.header.clone(),
            mailbox: mailbox.clone(),
            sender: self.sender.clone(),
            subject: self.subject.clone(),
        }
    }

    /// The steps that put this change back.
    pub fn undo(&self) -> Result<Vec<Step>, String> {
        if self.undone {
            return Err("That’s already undone.".into());
        }
        if matches!(self.why, Why::Undo { .. }) {
            return Err("That was an undo.".into());
        }
        match &self.change {
            Change::Mark { what, was, at, id, .. } => {
                Ok(vec![Step::Mark { target: self.target(at, *id), what: *what, value: *was }])
            }
            Change::Move { from, to, id } => Ok(vec![Step::Move { target: self.target(to, *id), to: from.clone() }]),
            Change::Junk { from, to, was, id } => {
                let mut steps = Vec::new();
                if from != to {
                    steps.push(Step::Move { target: self.target(to, *id), to: from.clone() });
                }
                // After a move Mail has renumbered it; the Message-ID finds it.
                let number = if from == to { *id } else { 0 };
                steps.push(Step::Mark { target: self.target(from, number), what: Mark::Junk, value: *was });
                Ok(steps)
            }
            Change::Draft { .. } | Change::Reply { .. } => Err("Drafts stay in Mail.".into()),
        }
    }
}

/// The last part of a mailbox path.
pub fn leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Log {
    pub entries: Vec<Entry>,
}

impl Log {
    /// The saved log, or an empty one when there is none or it doesn't read.
    pub fn load(path: &Path) -> Log {
        std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec(self).map_err(|_| "The mail log didn’t save.")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|_| "The mail log didn’t save.")?;
        }
        let partial = path.with_extension("partial");
        std::fs::write(&partial, bytes).map_err(|_| "The mail log didn’t save.")?;
        std::fs::rename(&partial, path).map_err(|_| "The mail log didn’t save.".to_string())
    }

    pub fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
        if self.entries.len() > KEEP {
            let extra = self.entries.len() - KEEP;
            self.entries.drain(..extra);
        }
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    pub fn mark_undone(&mut self, id: &str) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
            entry.undone = true;
        }
    }

    /// Newest first.
    pub fn recent(&self, count: usize) -> Vec<&Entry> {
        self.entries.iter().rev().take(count).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inbox() -> Mailbox {
        Mailbox { account: "iCloud".into(), path: "INBOX".into() }
    }

    fn mailbox(path: &str) -> Mailbox {
        Mailbox { account: "iCloud".into(), path: path.into() }
    }

    fn target() -> Target {
        Target { id: 14, header: "ana1@x".into(), mailbox: inbox(), sender: "Ana <ana@x.org>".into(), subject: "Dinner?".into() }
    }

    fn entry(change: Change) -> Entry {
        Entry::new(change, Some(&target()), "Dinner?", Why::Manual)
    }

    #[test]
    fn undoing_a_mark_sets_back_what_it_was_where_it_is() {
        let entry = entry(Change::Mark { what: Mark::Flagged, was: false, now: true, at: inbox(), id: 14 });
        assert_eq!(entry.label(), "Flagged · Dinner?");
        let steps = entry.undo().unwrap();
        assert_eq!(steps, vec![Step::Mark { target: target(), what: Mark::Flagged, value: false }]);
    }

    #[test]
    fn undoing_a_move_moves_it_back_from_where_it_went() {
        let entry = entry(Change::Move { from: inbox(), to: mailbox("Archive/2026"), id: 22 });
        assert_eq!(entry.label(), "To 2026 · Dinner?");
        match entry.undo().unwrap().as_slice() {
            [Step::Move { target, to }] => {
                assert_eq!(target.mailbox, mailbox("Archive/2026"));
                assert_eq!(target.id, 22);
                assert_eq!(target.header, "ana1@x");
                assert_eq!(to, &inbox());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn undoing_junk_moves_it_back_then_restores_its_junk_mark() {
        let moved = entry(Change::Junk { from: inbox(), to: mailbox("Junk"), was: false, id: 40 });
        let steps = moved.undo().unwrap();
        assert_eq!(steps.len(), 2);
        assert!(matches!(&steps[0], Step::Move { target, to } if target.mailbox == mailbox("Junk") && target.id == 40 && to == &inbox()));
        assert!(matches!(&steps[1], Step::Mark { target, what: Mark::Junk, value: false } if target.mailbox == inbox() && target.id == 0));
        // Junk left in place (mail on this Mac) only takes the mark back.
        let here = entry(Change::Junk { from: inbox(), to: inbox(), was: false, id: 14 });
        assert_eq!(here.undo().unwrap(), vec![Step::Mark { target: target(), what: Mark::Junk, value: false }]);
    }

    #[test]
    fn drafts_undos_and_undone_entries_are_never_undone() {
        let draft = Entry::new(Change::Draft { id: 51, to: vec!["ana@x.org".into()], attachments: vec!["p1.pdf".into()] }, None, "Payslip", Why::Plan);
        assert!(!draft.undoable() && draft.undo().is_err());
        assert_eq!(draft.label(), "Draft to ana@x.org · 1 📎 · Payslip");
        let mut done = entry(Change::Mark { what: Mark::Read, was: false, now: true, at: inbox(), id: 14 });
        assert!(done.undoable());
        done.undone = true;
        assert!(!done.undoable() && done.undo().is_err());
        let mut undo = entry(Change::Mark { what: Mark::Read, was: true, now: false, at: inbox(), id: 14 });
        undo.why = Why::Undo { of: "x".into() };
        assert!(!undo.undoable() && undo.undo().is_err());
    }

    #[test]
    fn the_log_keeps_the_newest_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail").join("log.json");
        let mut log = Log::default();
        for _ in 0..KEEP + 3 {
            log.push(entry(Change::Mark { what: Mark::Read, was: false, now: true, at: inbox(), id: 14 }));
        }
        assert_eq!(log.entries.len(), KEEP);
        let newest = log.recent(1)[0].id.clone();
        log.mark_undone(&newest);
        log.save(&path).unwrap();
        let loaded = Log::load(&path);
        assert_eq!(loaded, log);
        assert!(loaded.get(&newest).unwrap().undone);
        assert_eq!(Log::load(&dir.path().join("missing.json")), Log::default());
        std::fs::write(&path, b"not json").unwrap();
        assert_eq!(Log::load(&path), Log::default());
    }
}
