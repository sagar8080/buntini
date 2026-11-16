//! Line diffs for the changes an agent makes, from the tool input alone (the old and new
//! text an Edit carries, or a Write's whole file). Longest-common-subsequence on lines;
//! above `MAX_LINES` it only counts. After Coucou's DiffEngine (MIT).

/// Above this many lines on either side, only counts.
const MAX_LINES: usize = 4_000;
/// Unchanged lines kept around each change.
const CONTEXT: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Same,
    Add,
    Remove,
    /// Unchanged lines left out.
    Gap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: Kind,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    pub added: usize,
    pub removed: usize,
    /// Empty when the texts were too long to compare line by line.
    pub lines: Vec<Line>,
}

pub fn diff(old: &str, new: &str) -> Diff {
    let a: Vec<&str> = if old.is_empty() { Vec::new() } else { old.lines().collect() };
    let b: Vec<&str> = if new.is_empty() { Vec::new() } else { new.lines().collect() };
    if a.len() > MAX_LINES || b.len() > MAX_LINES {
        return counts(&a, &b);
    }
    // Trim the shared start and end first: edits are usually small.
    let start = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let end = a[start..].iter().rev().zip(b[start..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (am, bm) = (&a[start..a.len() - end], &b[start..b.len() - end]);
    let mut table = vec![vec![0u32; bm.len() + 1]; am.len() + 1];
    for i in (0..am.len()).rev() {
        for j in (0..bm.len()).rev() {
            table[i][j] = if am[i] == bm[j] { table[i + 1][j + 1] + 1 } else { table[i + 1][j].max(table[i][j + 1]) };
        }
    }
    let mut all: Vec<Line> = a[..start].iter().map(|text| Line { kind: Kind::Same, text: text.to_string() }).collect();
    let (mut i, mut j) = (0, 0);
    while i < am.len() || j < bm.len() {
        if i < am.len() && j < bm.len() && am[i] == bm[j] {
            all.push(Line { kind: Kind::Same, text: am[i].to_string() });
            i += 1;
            j += 1;
        } else if i < am.len() && (j == bm.len() || table[i + 1][j] >= table[i][j + 1]) {
            // Removed lines before the lines that replace them, as diffs usually read.
            all.push(Line { kind: Kind::Remove, text: am[i].to_string() });
            i += 1;
        } else {
            all.push(Line { kind: Kind::Add, text: bm[j].to_string() });
            j += 1;
        }
    }
    all.extend(a[a.len() - end..].iter().map(|text| Line { kind: Kind::Same, text: text.to_string() }));
    let added = all.iter().filter(|line| line.kind == Kind::Add).count();
    let removed = all.iter().filter(|line| line.kind == Kind::Remove).count();
    Diff { added, removed, lines: hunks(all) }
}

/// Counts only: lines in one text and not the other, by multiset.
fn counts(a: &[&str], b: &[&str]) -> Diff {
    let mut seen: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
    for line in a {
        *seen.entry(line).or_default() += 1;
    }
    for line in b {
        *seen.entry(line).or_default() -= 1;
    }
    let removed = seen.values().filter(|n| **n > 0).map(|n| *n as usize).sum();
    let added = seen.values().filter(|n| **n < 0).map(|n| n.unsigned_abs() as usize).sum();
    Diff { added, removed, lines: Vec::new() }
}

/// Keeps changed lines with `CONTEXT` lines around them; runs of unchanged lines in
/// between become one gap.
fn hunks(all: Vec<Line>) -> Vec<Line> {
    let near: Vec<bool> = (0..all.len())
        .map(|index| {
            let from = index.saturating_sub(CONTEXT);
            let to = (index + CONTEXT + 1).min(all.len());
            all[from..to].iter().any(|line| line.kind != Kind::Same)
        })
        .collect();
    let mut out = Vec::new();
    let mut skipped = false;
    for (line, keep) in all.into_iter().zip(near) {
        if keep {
            if skipped && !out.is_empty() {
                out.push(Line { kind: Kind::Gap, text: String::new() });
            }
            skipped = false;
            out.push(line);
        } else {
            skipped = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(diff: &Diff) -> String {
        diff.lines
            .iter()
            .map(|line| match line.kind {
                Kind::Same => '=',
                Kind::Add => '+',
                Kind::Remove => '-',
                Kind::Gap => '~',
            })
            .collect()
    }

    #[test]
    fn an_edit_counts_and_shows_its_lines() {
        let diff = diff("a\nb\nc\n", "a\nB\nc\nd\n");
        assert_eq!((diff.added, diff.removed), (2, 1));
        assert_eq!(kinds(&diff), "=-+=+");
        assert_eq!(diff.lines.iter().filter(|line| line.kind == Kind::Add).map(|line| line.text.as_str()).collect::<Vec<_>>(), ["B", "d"]);
    }

    #[test]
    fn a_new_file_is_all_added() {
        let diff = diff("", "one\ntwo\n");
        assert_eq!((diff.added, diff.removed), (2, 0));
        assert_eq!(kinds(&diff), "++");
    }

    #[test]
    fn far_apart_changes_are_shown_with_a_gap_between() {
        let old: String = (0..30).map(|n| format!("line {n}\n")).collect();
        let new = old.replace("line 2\n", "line two\n").replace("line 27\n", "line twenty-seven\n");
        let diff = diff(&old, &new);
        assert_eq!((diff.added, diff.removed), (2, 2));
        assert_eq!(diff.lines.iter().filter(|line| line.kind == Kind::Gap).count(), 1);
        assert!(diff.lines.len() < 16);
    }

    #[test]
    fn very_long_texts_are_only_counted() {
        let old: String = (0..5_000).map(|n| format!("{n}\n")).collect();
        let new = format!("{old}extra\n");
        let diff = diff(&old, &new);
        assert_eq!((diff.added, diff.removed), (1, 0));
        assert!(diff.lines.is_empty());
    }

    #[test]
    fn nothing_changed_is_nothing() {
        let diff = diff("same\n", "same\n");
        assert_eq!((diff.added, diff.removed), (0, 0));
        assert!(diff.lines.is_empty());
    }
}
