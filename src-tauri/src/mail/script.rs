//! The AppleScript the mail hub runs, and the one way it runs it.
//!
//! Every value reaches Mail as its own argument after `--`, never as script text, so a
//! subject, address or mailbox name can't change what a script does. Every script stops
//! before its first `tell` when Mail is closed, so the hub never opens Mail by itself.
//! Fields come back separated by tabs and records by line breaks; `clean` turns both into
//! spaces inside a field, so a value can't split a record.
use super::{Mailbox, Message};

/// What a script answers when Mail is closed.
pub const CLOSED: &str = "__buntini_mail_closed__";
pub const CLOSED_TEXT: &str = "Mail is closed.";
/// The longest single value handed to a script.
const MAX_ARGUMENT: usize = 64 * 1024;

const HELPERS: &str = r#"on two(n)
  return text -2 thru -1 of ("0" & (n as integer))
end two
on stamp(d)
  if d is missing value then return ""
  return ((year of d) as text) & "-" & my two(month of d) & "-" & my two(day of d) & " " & my two(hours of d) & ":" & my two(minutes of d) & ":" & my two(seconds of d)
end stamp
on clean(t)
  if t is missing value then return ""
  set AppleScript's text item delimiters to {tab, return, linefeed, character id 8232, character id 8233}
  set parts to text items of (t as text)
  set AppleScript's text item delimiters to " "
  set t to parts as text
  set AppleScript's text item delimiters to ""
  return t
end clean
on joined(xs, separator)
  set AppleScript's text item delimiters to separator
  set t to xs as text
  set AppleScript's text item delimiters to ""
  return t
end joined
-- A mailbox by its account's name and its path ("Work/Clients"); no account means On My Mac.
on box(acct, p)
  tell application "Mail"
    if acct is "" then
      if not (exists mailbox p) then error "There’s no mailbox called " & p & "." number 4004
      return mailbox p
    end if
    if not (exists mailbox p of account acct) then error "There’s no mailbox called " & p & " in " & acct & "." number 4004
    return mailbox p of account acct
  end tell
end box
on boxPath(mb)
  tell application "Mail"
    set p to name of mb
    set cur to mb
    repeat 12 times
      try
        -- A parent mailbox is a container; the account above the top one is neither.
        set cur to container of cur
        set k to class of cur
        if k is not mailbox and k is not container then exit repeat
        set p to (name of cur) & "/" & p
      on error
        exit repeat
      end try
    end repeat
  end tell
  return p
end boxPath
-- The email by Mail's number, checked against its Message-ID. Mail gives an email a new
-- number when it moves, so a missing number is looked up again by the Message-ID alone.
on findMessage(mb, n, h)
  tell application "Mail"
    try
      set m to first message of mb whose id is n
      if (message id of m) is h then return m
    end try
    set found to (messages of mb whose message id is h)
    if (count of found) is 1 then return item 1 of found
    if (count of found) is 0 then error "That email isn’t in " & (name of mb) & " any more." number 4001
    error "More than one email there has that Message-ID." number 4002
  end tell
end findMessage
-- Waits for an email to show in a mailbox after a move: its new number, or 0.
on arrival(mb, h)
  tell application "Mail"
    repeat 60 times
      set ids to id of (messages of mb whose message id is h)
      if (count of ids) > 0 then return item 1 of ids
      delay 0.1
    end repeat
  end tell
  return 0
end arrival
"#;

const START: &str = "on run argv\n  if application \"Mail\" is not running then return \"__buntini_mail_closed__\"\n";
const END: &str = "end run\n";

/// argv: how many of the newest emails to read from each mailbox, then pairs of account
/// and mailbox path. With no pairs, every enabled account's inbox.
/// Rows: received, number, Message-ID, read, flagged, junk, sender, subject, account, path.
pub const LIST: &str = r#"  set depth to (item 1 of argv) as integer
  set boxes to {}
  tell application "Mail"
    if (count of argv) > 1 then
      repeat with i from 2 to (count of argv) - 1 by 2
        set end of boxes to {item i of argv, item (i + 1) of argv}
      end repeat
    else
      repeat with a in (every account whose enabled is true)
        repeat with nm in {"INBOX", "Inbox"}
          if exists mailbox (nm as text) of a then
            set end of boxes to {name of a, nm as text}
            exit repeat
          end if
        end repeat
      end repeat
    end if
  end tell
  set out to {}
  repeat with pair in boxes
    set acct to item 1 of pair
    set mb to my box(acct, item 2 of pair)
    set p to my boxPath(mb)
    tell application "Mail"
      with timeout of 30 seconds
        set n to count of messages of mb
        if n > 0 then
          set k to n
          if k > depth then set k to depth
          -- Mail keeps the newest first in most mailboxes, but not in every one.
          set a to 1
          set z to k
          if n > 1 then
            if (date received of message 1 of mb) < (date received of message n of mb) then
              set a to n - k + 1
              set z to n
            end if
          end if
          set ids to id of messages a thru z of mb
          set hs to message id of messages a thru z of mb
          set ds to date received of messages a thru z of mb
          set rs to read status of messages a thru z of mb
          set fs to flagged status of messages a thru z of mb
          set js to junk mail status of messages a thru z of mb
          set ss to sender of messages a thru z of mb
          set ts to subject of messages a thru z of mb
          repeat with i from 1 to count of ids
            set end of out to my stamp(item i of ds) & tab & (item i of ids as text) & tab & my clean(item i of hs) & tab & (item i of rs as text) & tab & (item i of fs as text) & tab & (item i of js as text) & tab & my clean(item i of ss) & tab & my clean(item i of ts) & tab & my clean(acct) & tab & my clean(p)
          end repeat
        end if
      end timeout
    end tell
  end repeat
  return my joined(out, linefeed)
"#;

/// Every mailbox: account (empty for On My Mac) and path.
pub const MAILBOXES: &str = r#"  set out to {}
  tell application "Mail"
    repeat with mb in mailboxes
      set end of out to "" & tab & my clean(my boxPath(mb))
    end repeat
    repeat with a in (every account whose enabled is true)
      repeat with mb in mailboxes of a
        set end of out to my clean(name of a) & tab & my clean(my boxPath(mb))
      end repeat
    end repeat
  end tell
  return my joined(out, linefeed)
"#;

/// argv: account, path, number, Message-ID. One line of received, number, sender, subject,
/// To addresses, attachment names and the body's full length, then the body (at most
/// 20,000 characters).
pub const READ: &str = r#"  set mb to my box(item 1 of argv, item 2 of argv)
  set m to my findMessage(mb, (item 3 of argv) as integer, item 4 of argv)
  tell application "Mail"
    with timeout of 20 seconds
      set c to content of m
    end timeout
    if c is missing value then set c to ""
    set fullLength to length of c
    if fullLength > 20000 then set c to text 1 thru 20000 of c
    set toList to {}
    repeat with r in to recipients of m
      set end of toList to my clean(address of r)
    end repeat
    set names to {}
    try
      repeat with f in mail attachments of m
        set end of names to my clean(name of f)
      end repeat
    end try
    return my stamp(date received of m) & tab & (id of m as text) & tab & my clean(sender of m) & tab & my clean(subject of m) & tab & my joined(toList, ",") & tab & my joined(names, "/") & tab & (fullLength as text) & linefeed & c
  end tell
"#;

/// argv: account, path, number, Message-ID, `read`, `flagged` or `junk`, `true` or `false`.
/// Answers: before, after (read back from Mail), number.
pub const MARK: &str = r#"  set mb to my box(item 1 of argv, item 2 of argv)
  set h to item 4 of argv
  set m to my findMessage(mb, (item 3 of argv) as integer, h)
  set what to item 5 of argv
  set wanted to (item 6 of argv) is "true"
  tell application "Mail"
    if what is "read" then
      set was to read status of m
      set read status of m to wanted
    else if what is "flagged" then
      set was to flagged status of m
      set flagged status of m to wanted
    else if what is "junk" then
      set was to junk mail status of m
      set junk mail status of m to wanted
    else
      error "Mail can’t change that." number 4005
    end if
  end tell
  set m to my findMessage(mb, (item 3 of argv) as integer, h)
  tell application "Mail"
    if what is "read" then
      set now to read status of m
    else if what is "flagged" then
      set now to flagged status of m
    else
      set now to junk mail status of m
    end if
    return (was as text) & tab & (now as text) & tab & (id of m as text)
  end tell
"#;

/// argv: account, path, number, Message-ID, then the destination's account and path.
/// Answers: the email's new number there, how many copies are left behind, and the
/// destination's path as Mail names it.
pub const MOVE: &str = r#"  set mb to my box(item 1 of argv, item 2 of argv)
  set dest to my box(item 5 of argv, item 6 of argv)
  set h to item 4 of argv
  set m to my findMessage(mb, (item 3 of argv) as integer, h)
  tell application "Mail" to move m to dest
  set n to my arrival(dest, h)
  if n is 0 then error "Mail didn’t show the email in " & (item 6 of argv) & " after moving it." number 4003
  tell application "Mail" to set remaining to count of (messages of mb whose message id is h)
  return (n as text) & tab & (remaining as text) & tab & my clean(my boxPath(dest))
"#;

/// argv: account, path, number, Message-ID. Marks the email as junk and moves it to its
/// account's junk mailbox when the account has one; mail on this Mac stays where it is.
/// Answers: junk before, junk after (read back), destination account, path, new number.
pub const JUNK: &str = r#"  set acct to item 1 of argv
  set mb to my box(acct, item 2 of argv)
  set h to item 4 of argv
  set m to my findMessage(mb, (item 3 of argv) as integer, h)
  tell application "Mail"
    set was to junk mail status of m
    set junk mail status of m to true
    set dest to missing value
    if acct is not "" then
      repeat with nm in {"Junk", "Junk E-mail", "Junk Email", "Spam", "[Gmail]/Spam", "Bulk Mail"}
        if exists mailbox (nm as text) of account acct then
          set dest to mailbox (nm as text) of account acct
          exit repeat
        end if
      end repeat
    end if
  end tell
  set destPath to my boxPath(mb)
  if dest is not missing value then
    if my boxPath(dest) is not destPath then
      tell application "Mail" to move m to dest
      set n to my arrival(dest, h)
      if n is 0 then error "Mail didn’t show the email in Junk after moving it." number 4003
      set destPath to my boxPath(dest)
      set m to my findMessage(dest, n, h)
    end if
  end if
  tell application "Mail" to return (was as text) & tab & ((junk mail status of m) as text) & tab & my clean(acct) & tab & my clean(destPath) & tab & ((id of m) as text)
"#;

/// argv: To address (or empty), subject, body, `1` to open the draft on screen, then the
/// files to attach. Saves the draft and reads it back from the Drafts mailbox: its number,
/// subject, To addresses and attachment names. Nothing is sent.
pub const DRAFT: &str = r#"  set requestedSubject to item 2 of argv
  set shown to (item 4 of argv) is "1"
  tell application "Mail"
    set known to id of (every message of drafts mailbox whose subject is requestedSubject)
    set created to make new outgoing message with properties {subject:requestedSubject, content:(item 3 of argv) & return & return, visible:shown}
    if item 1 of argv is not "" then
      tell created to make new to recipient at end of to recipients with properties {address:item 1 of argv}
    end if
    if (count of argv) > 4 then
      repeat with i from 5 to count of argv
        tell content of created to make new attachment with properties {file name:(POSIX file (item i of argv))} at after last paragraph
      end repeat
      -- Attachments load in the background before a save keeps them.
      delay 1
    end if
    save created
    -- Mail can hold two copies for a moment while it saves again with the attachments, so
    -- the new draft counts once exactly one has held still for half a second.
    set newIDs to {}
    set steady to 0
    repeat 80 times
      set found to {}
      repeat with candidate in (every message of drafts mailbox whose subject is requestedSubject)
        if (id of candidate) is not in known then set end of found to (id of candidate)
      end repeat
      if (count of found) is 1 and found is newIDs then
        set steady to steady + 1
        if steady ≥ 2 then exit repeat
      else
        set steady to 0
      end if
      set newIDs to found
      delay 0.25
    end repeat
    if (count of newIDs) is not 1 or steady < 2 then error "Mail didn’t show the saved draft." number 4006
    set saved to first message of drafts mailbox whose id is (item 1 of newIDs)
    set toList to {}
    repeat with r in to recipients of saved
      set end of toList to my clean(address of r)
    end repeat
    set names to {}
    repeat with f in mail attachments of saved
      set end of names to my clean(name of f)
    end repeat
    if shown then activate
    return ((id of saved) as text) & tab & my clean(subject of saved) & tab & my joined(toList, ",") & tab & my joined(names, "/")
  end tell
"#;

/// The whole script for `body`, guarded so it returns `CLOSED` before touching a closed Mail.
pub fn source(body: &str) -> String {
    format!("{HELPERS}{START}{body}{END}")
}

/// A value as a script argument. Arguments are passed after `--`, so nothing in them is
/// read as an option or as script; only what a process argument can't hold is refused.
pub fn argument(value: &str) -> Result<String, String> {
    if value.contains('\0') {
        return Err("That has a character Mail can’t take.".into());
    }
    if value.len() > MAX_ARGUMENT {
        return Err("That’s too long to hand to Mail.".into());
    }
    Ok(value.to_string())
}

/// The osascript command for `body` with `argv`, every value after the option terminator.
pub fn command(body: &str, argv: &[String]) -> Result<tokio::process::Command, String> {
    let mut command = crate::actions::osascript_command("AppleScript", &source(body));
    for value in argv {
        command.arg(argument(value)?);
    }
    Ok(command)
}

/// Runs a script. `Err(CLOSED_TEXT)` when Mail is closed, so callers can tell that apart.
pub async fn run(body: &str, argv: &[String], seconds: u64) -> Result<String, String> {
    let output = crate::actions::output(command(body, argv)?, seconds).await.map_err(|error| tidy(&error))?;
    if output.trim() == CLOSED {
        return Err(CLOSED_TEXT.into());
    }
    Ok(output)
}

/// osascript's own error text, without its "execution error:" frame and character offsets.
fn tidy(error: &str) -> String {
    let text = error.strip_prefix("That didn’t work: ").unwrap_or(error);
    let text = text.split_once("execution error: ").map_or(text, |(_, rest)| rest);
    let text = text.strip_prefix("Mail got an error: ").unwrap_or(text);
    // "… (4001)" carries our own error numbers; Mail's are kept as they are.
    match text.rsplit_once(" (") {
        Some((message, number)) if number.trim_end_matches(')').parse::<i32>().is_ok_and(|n| (4001..=4099).contains(&n)) => message.to_string(),
        _ => text.to_string(),
    }
}

pub fn closed(error: &str) -> bool {
    error == CLOSED_TEXT
}

fn fields(line: &str) -> Vec<&str> {
    line.split('\t').map(str::trim).collect()
}

/// "2026-11-16 09:22:29" in this Mac's time zone, as Unix seconds.
pub fn seconds(stamp: &str) -> Option<i64> {
    use chrono::TimeZone;
    let at = chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S").ok()?;
    chrono::Local.from_local_datetime(&at).earliest().map(|at| at.timestamp())
}

fn flag(value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err("Mail answered with something unexpected.".into()),
    }
}

/// `LIST`'s rows. A row that doesn't read is an error, not a gap.
pub fn parse_list(output: &str) -> Result<Vec<Message>, String> {
    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let row = fields(line);
            if row.len() != 10 {
                return Err("Mail answered with an incomplete email.".to_string());
            }
            Ok(Message {
                id: row[1].parse().map_err(|_| "Mail answered without an email's number.")?,
                header: row[2].to_string(),
                at: seconds(row[0]).unwrap_or(0),
                read: flag(row[3])?,
                flagged: flag(row[4])?,
                junk: flag(row[5])?,
                sender: row[6].to_string(),
                subject: row[7].to_string(),
                mailbox: Mailbox { account: row[8].to_string(), path: row[9].to_string() },
            })
        })
        .collect()
}

pub fn parse_mailboxes(output: &str) -> Vec<Mailbox> {
    let mut boxes: Vec<Mailbox> = output
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(account, path)| Mailbox { account: account.trim().to_string(), path: path.trim().to_string() })
        .filter(|mailbox| !mailbox.path.is_empty())
        .collect();
    boxes.dedup();
    boxes
}

/// An email read in full.
#[derive(Debug, Clone, PartialEq)]
pub struct Opened {
    pub id: i64,
    pub at: i64,
    pub sender: String,
    pub subject: String,
    pub to: Vec<String>,
    pub attachments: Vec<String>,
    pub body: String,
    pub cut: bool,
}

pub fn parse_read(output: &str) -> Result<Opened, String> {
    let (head, body) = output.split_once('\n').unwrap_or((output, ""));
    let row = fields(head);
    if row.len() != 7 {
        return Err("Mail answered with an incomplete email.".into());
    }
    let length: usize = row[6].parse().map_err(|_| "Mail answered with an incomplete email.")?;
    let body = body.trim_end_matches('\n').replace("\r\n", "\n").replace('\r', "\n");
    Ok(Opened {
        id: row[1].parse().map_err(|_| "Mail answered without an email's number.")?,
        at: seconds(row[0]).unwrap_or(0),
        sender: row[2].to_string(),
        subject: row[3].to_string(),
        to: row[4].split(',').filter(|a| !a.is_empty()).map(str::to_string).collect(),
        attachments: row[5].split('/').filter(|a| !a.is_empty()).map(str::to_string).collect(),
        cut: length > body.chars().count(),
        body,
    })
}

/// `MARK`'s answer: before, after, number.
pub fn parse_mark(output: &str) -> Result<(bool, bool, i64), String> {
    let row = fields(output.trim());
    match row.as_slice() {
        [was, now, id] => Ok((flag(was)?, flag(now)?, id.parse().map_err(|_| "Mail answered without the email's number.")?)),
        _ => Err("Mail didn’t say what changed.".into()),
    }
}

/// `MOVE`'s answer: new number, copies left behind, destination path.
pub fn parse_move(output: &str) -> Result<(i64, usize, String), String> {
    let row = fields(output.trim());
    match row.as_slice() {
        [id, left, path] => Ok((
            id.parse().map_err(|_| "Mail answered without the email's number.")?,
            left.parse().map_err(|_| "Mail didn’t say whether the email moved.")?,
            path.to_string(),
        )),
        _ => Err("Mail didn’t say where the email went.".into()),
    }
}

/// `JUNK`'s answer: junk before, junk after, where it is now, its number there.
pub fn parse_junk(output: &str) -> Result<(bool, bool, Mailbox, i64), String> {
    let row = fields(output.trim());
    match row.as_slice() {
        [was, now, account, path, id] => Ok((
            flag(was)?,
            flag(now)?,
            Mailbox { account: account.to_string(), path: path.to_string() },
            id.parse().map_err(|_| "Mail answered without the email's number.")?,
        )),
        _ => Err("Mail didn’t say where the email went.".into()),
    }
}

/// `DRAFT`'s answer: the saved draft's number, subject, To addresses and attachments.
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    pub id: i64,
    pub subject: String,
    pub to: Vec<String>,
    pub attachments: Vec<String>,
}

pub fn parse_draft(output: &str) -> Result<Saved, String> {
    let row = fields(output.trim_end_matches(['\n', '\r']));
    if row.len() != 4 {
        return Err("Mail didn’t say which draft it saved.".into());
    }
    Ok(Saved {
        id: row[0].parse().map_err(|_| "Mail didn’t say which draft it saved.")?,
        subject: row[1].to_string(),
        to: row[2].split(',').filter(|a| !a.is_empty()).map(str::to_string).collect(),
        attachments: row[3].split('/').filter(|a| !a.is_empty()).map(str::to_string).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_value_is_an_argument_after_the_option_terminator() {
        let values = ["-e", "do shell script \"touch /tmp/x\"", "-l", "JavaScript", "--", "\"; quit"].map(String::from);
        let command = command(MARK, &values).unwrap();
        let args: Vec<String> = command.as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        // osascript -l AppleScript -e <source> -- <values…>
        assert_eq!(&args[..2], ["-l", "AppleScript"]);
        assert_eq!(args[2], "-e");
        assert_eq!(args[4], "--");
        assert_eq!(&args[5..], values);
        // The source is fixed text: no value was written into it.
        assert_eq!(args[3], source(MARK));
        assert!(!args[3].contains("touch /tmp/x"));
    }

    #[test]
    fn values_a_process_argument_cannot_carry_are_refused() {
        assert!(argument("Re: lunch\0").is_err());
        assert!(argument(&"x".repeat(MAX_ARGUMENT + 1)).is_err());
        assert_eq!(argument("Ana \"Bo\" O'Neil \\ tab\there\nnew line — ünï").unwrap(), "Ana \"Bo\" O'Neil \\ tab\there\nnew line — ünï");
        assert!(command(MARK, &["a\0b".into()]).is_err());
    }

    #[test]
    fn every_script_stops_before_touching_a_closed_mail() {
        for body in [LIST, MAILBOXES, READ, MARK, MOVE, JUNK, DRAFT] {
            let source = source(body);
            let guard = source.find("if application \"Mail\" is not running then return").unwrap();
            let run = source.find("on run argv").unwrap();
            // The guard is the first line of the run handler, before any tell.
            assert_eq!(&source[run..guard], "on run argv\n  ");
            assert!(!body.contains("run script") && !body.contains("do shell script"));
            // No value is ever spliced into script text.
            assert!(!body.contains("& \"\\\"\""));
        }
    }

    /// Runs the real AppleScript interpreter with no app involved: what goes in as an
    /// argument comes back exactly, whatever it holds.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn tricky_values_reach_applescript_unchanged() {
        let values = [
            "Ana \"Bo\" O'Neil",
            "back\\slash \\\" quote",
            "-e",
            "--",
            "\" & (do shell script \"echo pwned\") & \"",
            "tab\tand\nnew line\r",
            "Ünïcödé — 日本語 ✉️",
            "",
        ]
        .map(String::from);
        // Joined with a mark none of the values holds, and counted, so nothing can merge.
        let echo = "on run argv\n  set AppleScript's text item delimiters to \"␞\"\n  set t to ((count of argv) as text) & \"␞\" & (argv as text)\n  set AppleScript's text item delimiters to \"\"\n  return t\nend run\n";
        let mut command = crate::actions::osascript_command("AppleScript", echo);
        for value in &values {
            command.arg(argument(value).unwrap());
        }
        let output = crate::actions::output(command, 20).await.unwrap();
        let expected = format!("{}␞{}\n", values.len(), values.join("␞"));
        assert_eq!(output, expected);
    }

    #[test]
    fn rows_read_with_their_mailbox_and_reject_broken_lines() {
        let output = "2026-11-16 09:22:29\t14\tana1@x\tfalse\ttrue\tfalse\tAna <ana@example.org>\tDinner?\t\tImport/Test\n";
        let rows = parse_list(output).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, 14);
        assert!(!rows[0].read && rows[0].flagged && !rows[0].junk);
        assert_eq!(rows[0].mailbox, Mailbox { account: String::new(), path: "Import/Test".into() });
        assert!(rows[0].at > 1_700_000_000);
        assert!(parse_list("2026-11-16 09:22:29\t14\tana1@x\tmaybe\ttrue\tfalse\tA\tB\t\tC").is_err());
        assert!(parse_list("only\tthree\tfields").is_err());
        assert_eq!(parse_list("\n").unwrap(), vec![]);
    }

    #[test]
    fn answers_read_back() {
        assert_eq!(parse_mark("false\ttrue\t22\n").unwrap(), (false, true, 22));
        assert!(parse_mark("false\ttrue").is_err());
        assert_eq!(parse_move("23\t0\tArchive\n").unwrap(), (23, 0, "Archive".into()));
        let (was, now, at, id) = parse_junk("false\ttrue\tiCloud\tJunk\t40").unwrap();
        assert!(!was && now && id == 40);
        assert_eq!(at, Mailbox { account: "iCloud".into(), path: "Junk".into() });
        let saved = parse_draft("51\tPayslip\tana@example.org\tPayslip - Ana.pdf\n").unwrap();
        assert_eq!(saved.to, ["ana@example.org"]);
        assert_eq!(saved.attachments, ["Payslip - Ana.pdf"]);
        let opened = parse_read("2026-11-16 09:22:29\t14\tAna\tHi\tme@x.org\t\t11\nHello\r\nthere").unwrap();
        assert_eq!(opened.body, "Hello\nthere");
        assert!(!opened.cut && opened.attachments.is_empty());
    }

    #[test]
    fn our_error_numbers_leave_just_the_sentence() {
        assert_eq!(
            tidy("That didn’t work: 120:190: execution error: Mail got an error: That email isn’t in Inbox any more. (4001)"),
            "That email isn’t in Inbox any more."
        );
        assert_eq!(tidy("123:130: execution error: Mail got an error: AppleEvent timed out. (-1712)"), "AppleEvent timed out. (-1712)");
    }
}
