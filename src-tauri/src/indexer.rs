//! Explicit, bounded FTS5 indexing. Stored text selects candidates only: every
//! result is independently reopened inside current folder permissions before
//! any metadata or excerpt is returned to the user or a local model.

use crate::{local, types::FileHit};
use rusqlite::{params, Connection};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use walkdir::{DirEntry, WalkDir};

pub const MAX_ENTRIES: usize = 10_000;
pub const MAX_DOCUMENTS: usize = 5_000;
pub const MAX_CONTENT_BYTES: usize = 20 * 1024 * 1024;
const MAX_DEPTH: usize = 20;
const MAX_HITS: usize = 40;

#[derive(Debug, Clone)]
pub struct IndexedDoc {
    pub root: String,
    pub path: String,
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct IndexBuild {
    pub docs: Vec<IndexedDoc>,
    pub scanned_entries: usize,
    pub content_bytes: usize,
    pub skipped: usize,
    pub truncated: bool,
    pub built_at: String,
}

#[derive(Debug, Clone)]
pub struct IndexCandidate {
    pub root: String,
    pub path: String,
}

fn check_cancel(cancel: &Arc<AtomicBool>) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Task cancelled.".into())
    } else {
        Ok(())
    }
}

fn visible(entry: &DirEntry) -> bool {
    entry.depth() == 0 || !entry.file_name().to_string_lossy().starts_with('.')
}

fn current_root(root: &Path) -> bool {
    root.is_absolute() && root.canonicalize().ok().as_deref() == Some(root)
}

/// Run only after the user explicitly requests a rebuild. Collection is pure
/// filesystem work and belongs in spawn_blocking, without holding the DB lock.
/// Documents are read through the Apple helper (`local::read_content`); text past
/// `local::MAX_READ` is cut there, so every document fits the same per-file limit.
pub fn rebuild(
    roots: &[String],
    documents: &local::Documents,
    cancel: &Arc<AtomicBool>,
) -> Result<IndexBuild, String> {
    let mut build = IndexBuild {
        docs: Vec::new(),
        scanned_entries: 0,
        content_bytes: 0,
        skipped: 0,
        truncated: false,
        built_at: String::new(),
    };
    let mut visited_roots = HashSet::new();
    'roots: for root_string in roots {
        check_cancel(cancel)?;
        if !visited_roots.insert(root_string) {
            continue;
        }
        let root = PathBuf::from(root_string);
        if !current_root(&root) {
            build.skipped += 1;
            continue;
        }
        for item in WalkDir::new(&root)
            .follow_links(false)
            .max_depth(MAX_DEPTH)
            .into_iter()
            .filter_entry(visible)
        {
            check_cancel(cancel)?;
            if build.scanned_entries >= MAX_ENTRIES || build.docs.len() >= MAX_DOCUMENTS {
                build.truncated = true;
                break 'roots;
            }
            build.scanned_entries += 1;
            let entry = match item {
                Ok(entry) => entry,
                Err(_) => {
                    build.skipped += 1;
                    continue;
                }
            };
            if entry.depth() == MAX_DEPTH && entry.file_type().is_dir() {
                build.truncated = true;
            }
            if !entry.file_type().is_file() || entry.file_type().is_symlink() {
                continue;
            }
            let Some(path) = entry.path().to_str() else {
                build.skipped += 1;
                continue;
            };
            let Some(name) = entry.file_name().to_str() else {
                build.skipped += 1;
                continue;
            };
            let mut file = match local::scoped_open(&root, entry.path()) {
                Ok(file) => file,
                Err(_) => {
                    build.skipped += 1;
                    continue;
                }
            };
            let content = local::read_content(&mut file, entry.path(), documents);
            if build.content_bytes.saturating_add(content.len()) > MAX_CONTENT_BYTES {
                build.truncated = true;
                break 'roots;
            }
            build.content_bytes += content.len();
            // Overlapping authorized folders keep separate root associations.
            // Revoking either one must not erase another still-allowed scope.
            build.docs.push(IndexedDoc {
                root: root_string.clone(),
                path: path.into(),
                name: name.into(),
                content,
            });
        }
    }
    check_cancel(cancel)?;
    build.built_at = crate::types::now();
    Ok(build)
}

/// Caller must hold the store lock and pass freshly read authorized roots.
/// Revoked roots are filtered before any content is persisted. The old index
/// remains intact if validation/insertion fails because replacement is atomic.
/// Returns the number of distinct files; overlapping folder bindings are not
/// double-counted in the user-visible index status.
pub fn replace_index(
    db: &mut Connection,
    current_roots: &[String],
    docs: &[IndexedDoc],
) -> Result<usize, String> {
    if docs.len() > MAX_DOCUMENTS {
        return Err("The index exceeds its document limit.".into());
    }
    let roots = current_roots
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut accepted = Vec::new();
    let mut seen = HashSet::new();
    let mut bytes = 0_usize;
    for doc in docs {
        if !roots.contains(doc.root.as_str()) {
            continue;
        }
        let root = Path::new(&doc.root);
        let path = Path::new(&doc.path);
        if !current_root(root) || local::scoped_open(root, path).is_err() {
            continue;
        }
        if doc.content.len() > local::MAX_READ as usize
            || doc.content.contains('\0')
            || path.file_name().and_then(|name| name.to_str()) != Some(doc.name.as_str())
        {
            return Err("An indexed document failed scope or size validation.".into());
        }
        if !seen.insert((doc.root.as_str(), doc.path.as_str())) {
            continue;
        }
        bytes = bytes.saturating_add(doc.content.len());
        if bytes > MAX_CONTENT_BYTES {
            return Err("The index exceeds its text storage limit.".into());
        }
        accepted.push(doc);
    }
    let transaction = db.transaction().map_err(|error| error.to_string())?;
    transaction
        .execute("DELETE FROM file_index", [])
        .map_err(|error| error.to_string())?;
    {
        let mut insert = transaction
            .prepare("INSERT INTO file_index(root,path,name,content) VALUES(?1,?2,?3,?4)")
            .map_err(|error| error.to_string())?;
        for doc in &accepted {
            insert
                .execute(params![doc.root, doc.path, doc.name, doc.content])
                .map_err(|error| error.to_string())?;
        }
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(accepted
        .iter()
        .map(|doc| doc.path.as_str())
        .collect::<HashSet<_>>()
        .len())
}

fn terms(query: &str) -> Result<Vec<(String, Option<String>)>, String> {
    if query.trim().is_empty() || query.len() > 512 {
        return Err("Enter a file query of up to 512 characters.".into());
    }
    let mut result = Vec::new();
    for word in query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        let alternative = local::singular_search_term(word);
        let word = word.to_lowercase();
        if !result.iter().any(|(existing, _)| existing == &word) {
            result.push((word, alternative));
        }
        if result.len() > 32 {
            return Err("Use up to 32 search words for the file index.".into());
        }
    }
    if result.is_empty() {
        return Err("Enter a file name or search words containing letters or numbers.".into());
    }
    Ok(result)
}

fn matches_terms(
    name: &str,
    content: &str,
    words: &[(String, Option<String>)],
    allow_alternatives: bool,
) -> bool {
    let name = name.to_lowercase();
    let content = content.to_lowercase();
    words.iter().all(|(term, alternative)| {
        let matches = |word: &str| {
            word.starts_with(term)
                || (allow_alternatives
                    && alternative
                        .as_ref()
                        .is_some_and(|term| word.starts_with(term)))
        };
        name.split(|c: char| !c.is_alphanumeric()).any(matches)
            || content.split(|c: char| !c.is_alphanumeric()).any(matches)
    })
}

/// Retrieve paths only, never stored excerpts. Controller may release its DB
/// lock before verify_candidates performs any filesystem reads.
pub fn indexed_candidates(
    db: &Connection,
    roots: &[String],
    query: &str,
) -> Result<Vec<IndexCandidate>, String> {
    let words = terms(query)?;
    let expression = words
        .iter()
        .map(|(term, alternative)| {
            let literal = format!("\"{}\"*", term.replace('"', "\"\""));
            match alternative {
                Some(alternative) => format!("({literal} OR \"{alternative}\"*)"),
                None => literal,
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    let approved = roots.iter().map(String::as_str).collect::<HashSet<_>>();
    // The two unindexed columns have zero weight; filename matches rank ahead
    // of ordinary document content. No SQL or FTS operators come from the user.
    let mut statement = db.prepare("SELECT root,path FROM file_index WHERE file_index MATCH ?1 ORDER BY bm25(file_index,0.0,0.0,6.0,1.0), path LIMIT ?2")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![expression, MAX_DOCUMENTS as i64], |row| {
            Ok(IndexCandidate {
                root: row.get(0)?,
                path: row.get(1)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut result = Vec::new();
    for row in rows {
        let candidate = row.map_err(|error| error.to_string())?;
        if approved.contains(candidate.root.as_str()) {
            result.push(candidate);
        }
    }
    Ok(result)
}

pub fn verify_candidates(
    roots: &[String],
    candidates: &[IndexCandidate],
    query: &str,
    documents: &local::Documents,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<FileHit>, String> {
    verify_candidate_group(roots, candidates, query, documents, cancel).map(|group| group.hits)
}

/// Candidates are reread now, so a document's excerpt is its current text, not what
/// the index stored. Unchanged documents come from `local::read_content`'s memory.
pub(crate) fn verify_candidate_group(
    roots: &[String],
    candidates: &[IndexCandidate],
    query: &str,
    documents: &local::Documents,
    cancel: &Arc<AtomicBool>,
) -> Result<local::MatchGroup<FileHit>, String> {
    let words = terms(query)?;
    let approved = roots.iter().map(String::as_str).collect::<HashSet<_>>();
    let mut paths = HashSet::new();
    let mut hits = Vec::new();
    let mut fallback = Vec::new();
    for candidate in candidates.iter().take(MAX_DOCUMENTS) {
        check_cancel(cancel)?;
        if !approved.contains(candidate.root.as_str()) || !paths.insert(candidate.path.as_str()) {
            continue;
        }
        let root = Path::new(&candidate.root);
        if !current_root(root) {
            continue;
        }
        let path = Path::new(&candidate.path);
        let mut file = match local::scoped_open(root, path) {
            Ok(file) => file,
            Err(_) => continue,
        };
        let metadata = match file.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let content = local::read_content(&mut file, path, documents);
        let exact = matches_terms(name, &content, &words, false);
        if !exact
            && (!hits.is_empty()
                || fallback.len() >= MAX_HITS
                || !matches_terms(name, &content, &words, true))
        {
            continue;
        }
        let mut excerpt = content
            .lines()
            .filter(|line| {
                matches_terms("", line, &words, !exact)
                    || words.iter().any(|(term, alternative)| {
                        line.to_lowercase()
                            .split(|c: char| !c.is_alphanumeric())
                            .any(|word| {
                                word.starts_with(term)
                                    || (!exact
                                        && alternative
                                            .as_ref()
                                            .is_some_and(|term| word.starts_with(term)))
                            })
                    })
            })
            .take(3)
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(600)
            .collect::<String>();
        if excerpt.is_empty() {
            excerpt = content
                .lines()
                .filter(|line| !line.trim().is_empty())
                .take(3)
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(600)
                .collect();
        }
        let hit = FileHit {
            name: name.into(),
            path: candidate.path.clone(),
            size: metadata.len(),
            modified_at: metadata
                .modified()
                .ok()
                .map(chrono::DateTime::<chrono::Utc>::from)
                .map(|time| time.to_rfc3339())
                .unwrap_or_default(),
            excerpt: (!excerpt.is_empty()).then_some(excerpt),
        };
        if exact {
            fallback.clear();
            hits.push(hit);
        } else {
            fallback.push(hit);
        }
        if hits.len() == MAX_HITS {
            break;
        }
    }
    check_cancel(cancel)?;
    Ok(local::prefer_literal(hits, fallback))
}

pub fn query_index(
    db: &Connection,
    roots: &[String],
    query: &str,
    documents: &local::Documents,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<FileHit>, String> {
    check_cancel(cancel)?;
    let candidates = indexed_candidates(db, roots, query)?;
    verify_candidates(roots, &candidates, query, documents, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;
    // Plain-text index tests; document tests pass their own reader.
    fn rebuild(roots: &[String], cancel: &Arc<AtomicBool>) -> Result<IndexBuild, String> {
        super::rebuild(roots, &local::Documents::none(), cancel)
    }
    fn verify_candidates(
        roots: &[String],
        candidates: &[IndexCandidate],
        query: &str,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Vec<FileHit>, String> {
        super::verify_candidates(roots, candidates, query, &local::Documents::none(), cancel)
    }
    fn query_index(
        db: &Connection,
        roots: &[String],
        query: &str,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Vec<FileHit>, String> {
        super::query_index(db, roots, query, &local::Documents::none(), cancel)
    }
    fn token() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }
    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE VIRTUAL TABLE file_index USING fts5(path UNINDEXED,root UNINDEXED,name,content);").unwrap();
        db
    }
    fn root(path: &Path) -> String {
        path.canonicalize().unwrap().to_str().unwrap().into()
    }

    #[test]
    fn fts_prefix_queries_rank_names_and_return_only_fresh_excerpts() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(
            folder.path().join("invoice-summary.txt"),
            "Current financial notes.",
        )
        .unwrap();
        std::fs::write(
            folder.path().join("other.txt"),
            "An invoice appears in this document.",
        )
        .unwrap();
        let roots = vec![root(folder.path())];
        let build = rebuild(&roots, &token()).unwrap();
        let mut db = database();
        assert_eq!(replace_index(&mut db, &roots, &build.docs).unwrap(), 2);
        let hits = query_index(&db, &roots, "invoi", &token()).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].name, "invoice-summary.txt");
        assert_eq!(hits[0].excerpt.as_deref(), Some("Current financial notes."));
        std::fs::write(
            folder.path().join("other.txt"),
            "An invoice with fresh evidence.",
        )
        .unwrap();
        let hits = query_index(&db, &roots, "invoice", &token()).unwrap();
        assert!(hits
            .iter()
            .find(|hit| hit.name == "other.txt")
            .unwrap()
            .excerpt
            .as_deref()
            .unwrap()
            .contains("fresh evidence"));
        // Operators/punctuation become quoted words, never FTS syntax or SQL.
        assert!(query_index(&db, &roots, "invoice OR root:* --", &token())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn plural_candidates_are_fresh_and_exact_results_take_precedence() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(
            folder.path().join("offer.txt"),
            "ACME-6279 quote total 680 including assembly",
        )
        .unwrap();
        std::fs::write(
            folder.path().join("name.txt"),
            "Atlas Polaris BUS AcmeOps ACME-6279s",
        )
        .unwrap();
        let roots = vec![root(folder.path())];
        let build = rebuild(&roots, &token()).unwrap();
        let mut db = database();
        replace_index(&mut db, &roots, &build.docs).unwrap();
        let hits = query_index(&db, &roots, "ACME-6279 Quotes", &token()).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0]
            .excerpt
            .as_deref()
            .unwrap()
            .contains("680 including assembly"));
        assert!(query_index(&db, &roots, "ACME-6280 Quotes", &token())
            .unwrap()
            .is_empty());
        assert!(query_index(&db, &[], "quotes", &token())
            .unwrap()
            .is_empty());
        assert!(query_index(&db, &roots, "quotes", &Arc::new(AtomicBool::new(true))).is_err());
        for name in ["Atlas", "Polaris", "BUS", "AcmeOps", "ACME-6279s"] {
            let hits = query_index(&db, &roots, name, &token()).unwrap();
            assert_eq!(hits.len(), 1, "{name}");
            assert_eq!(hits[0].name, "name.txt");
        }
        // Reopening rejects a stale singular match, even though it is still
        // eligible in the stored FTS candidate set.
        std::fs::write(folder.path().join("offer.txt"), "unrelated replacement").unwrap();
        assert!(query_index(&db, &roots, "quotes", &token())
            .unwrap()
            .is_empty());
        assert!(query_index(&db, &[], "quotes", &token())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn exact_candidates_after_forty_plural_alternatives_are_not_discarded() {
        let folder = tempfile::tempdir().unwrap();
        let roots = vec![root(folder.path())];
        let mut candidates = Vec::new();
        for index in 0..MAX_HITS + 5 {
            let path = Path::new(&roots[0]).join(format!("offer-{index}.txt"));
            std::fs::write(&path, "vendor quote total 680").unwrap();
            candidates.push(IndexCandidate {
                root: roots[0].clone(),
                path: path.to_string_lossy().into_owned(),
            });
        }
        assert_eq!(
            verify_candidates(&roots, &candidates, "quotes", &token())
                .unwrap()
                .len(),
            MAX_HITS
        );
        let path = Path::new(&roots[0]).join("literal.txt");
        std::fs::write(&path, "vendor quotes total 745").unwrap();
        candidates.push(IndexCandidate {
            root: roots[0].clone(),
            path: path.to_string_lossy().into_owned(),
        });
        let hits = verify_candidates(&roots, &candidates, "quotes", &token()).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "literal.txt");

        let build = rebuild(&roots, &token()).unwrap();
        let mut db = database();
        replace_index(&mut db, &roots, &build.docs).unwrap();
        let hits = query_index(&db, &roots, "quotes", &token()).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "literal.txt");
    }

    #[test]
    fn revoked_deleted_and_changed_files_never_return_stored_private_text() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("notes.txt");
        std::fs::write(&file, "secretneedle old private text").unwrap();
        let roots = vec![root(folder.path())];
        let build = rebuild(&roots, &token()).unwrap();
        let mut db = database();
        replace_index(&mut db, &roots, &build.docs).unwrap();
        assert!(query_index(&db, &[], "secretneedle", &token())
            .unwrap()
            .is_empty());
        std::fs::write(&file, "unrelated replacement").unwrap();
        assert!(query_index(&db, &roots, "secretneedle", &token())
            .unwrap()
            .is_empty());
        std::fs::remove_file(&file).unwrap();
        assert!(query_index(&db, &roots, "secretneedle", &token())
            .unwrap()
            .is_empty());
        assert_eq!(replace_index(&mut db, &[], &build.docs).unwrap(), 0);
        let count: i64 = db
            .query_row("SELECT count(*) FROM file_index", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[cfg(unix)]
    #[test]
    fn hidden_files_symlink_swaps_and_out_of_scope_index_rows_are_denied() {
        use std::os::unix::fs::symlink;
        let folder = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("visible.txt"), "needle authorized").unwrap();
        std::fs::write(folder.path().join(".hidden.txt"), "needle hidden").unwrap();
        std::fs::write(outside.path().join("private.txt"), "needle outside").unwrap();
        symlink(outside.path(), folder.path().join("link")).unwrap();
        let roots = vec![root(folder.path())];
        let build = rebuild(&roots, &token()).unwrap();
        assert_eq!(build.docs.len(), 1);
        let mut db = database();
        replace_index(&mut db, &roots, &build.docs).unwrap();
        std::fs::remove_file(folder.path().join("visible.txt")).unwrap();
        symlink(
            outside.path().join("private.txt"),
            folder.path().join("visible.txt"),
        )
        .unwrap();
        for query in ["needle", "needles"] {
            assert!(query_index(&db, &roots, query, &token())
                .unwrap()
                .is_empty());
        }
        db.execute("INSERT INTO file_index(root,path,name,content) VALUES(?1,?2,'private.txt','needle outside')", params![roots[0], outside.path().join("private.txt").to_str().unwrap()]).unwrap();
        for query in ["needle", "needles"] {
            assert!(query_index(&db, &roots, query, &token())
                .unwrap()
                .is_empty());
        }
    }

    #[test]
    fn overlapping_roots_keep_independent_scope_and_binary_large_content_is_not_indexed() {
        let folder = tempfile::tempdir().unwrap();
        let nested = folder.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("notes.txt"), "needle shared scope").unwrap();
        std::fs::write(folder.path().join("binary.txt"), b"needle\0binary").unwrap();
        std::fs::write(
            folder.path().join("large.txt"),
            vec![b'x'; local::MAX_READ as usize + 1],
        )
        .unwrap();
        let roots = vec![root(folder.path()), root(&nested)];
        let build = rebuild(&roots, &token()).unwrap();
        assert_eq!(
            build
                .docs
                .iter()
                .filter(|doc| doc.name == "notes.txt")
                .count(),
            2
        );
        assert!(build
            .docs
            .iter()
            .filter(|doc| doc.name != "notes.txt")
            .all(|doc| doc.content.is_empty()));
        let mut db = database();
        replace_index(&mut db, &roots, &build.docs).unwrap();
        let hits = query_index(&db, &roots, "needle", &token()).unwrap();
        assert_eq!(hits.len(), 1);
        let still_allowed = vec![roots[1].clone()];
        assert_eq!(
            query_index(&db, &still_allowed, "needle", &token())
                .unwrap()
                .len(),
            1
        );
        assert!(rebuild(&roots, &Arc::new(AtomicBool::new(true))).is_err());
    }

    #[test]
    fn content_budget_is_enforced_and_incomplete_builds_are_marked() {
        let folder = tempfile::tempdir().unwrap();
        let block = vec![b'a'; local::MAX_READ as usize];
        let limit_files = MAX_CONTENT_BYTES / block.len();
        for index in 0..=limit_files {
            std::fs::write(folder.path().join(format!("document-{index}.txt")), &block).unwrap();
        }
        let build = rebuild(&[root(folder.path())], &token()).unwrap();
        assert!(build.truncated);
        assert_eq!(build.content_bytes, MAX_CONTENT_BYTES);
        assert_eq!(build.docs.len(), limit_files);
        assert!(build.scanned_entries <= MAX_ENTRIES);
        let nested = tempfile::tempdir().unwrap();
        let mut path = nested.path().to_path_buf();
        for _ in 0..=MAX_DEPTH {
            path.push("child");
            std::fs::create_dir(&path).unwrap();
        }
        std::fs::write(path.join("unreachable.txt"), "content beyond depth limit").unwrap();
        let build = rebuild(&[root(nested.path())], &token()).unwrap();
        assert!(build.truncated);
        assert!(build.docs.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn documents_are_indexed_by_their_text_and_reread_when_found() {
        use std::os::unix::fs::PermissionsExt;
        let tools = tempfile::tempdir().unwrap();
        let helper = tools.path().join("buntini-fm");
        // Answers with the piped bytes as the document's text.
        std::fs::write(
            &helper,
            "#!/bin/sh\nbody=$(cat)\nprintf '{\"text\":\"%s\",\"truncated\":false}\\n' \"$body\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let folder = tempfile::tempdir().unwrap();
        let pdf = folder.path().join("statement.pdf");
        std::fs::write(&pdf, "Deposit returned 1200").unwrap();
        let roots = vec![root(folder.path())];
        let documents = local::Documents::for_index(&helper, &token());
        let build = super::rebuild(&roots, &documents, &token()).unwrap();
        assert_eq!(build.docs[0].content, "Deposit returned 1200");
        let mut db = database();
        replace_index(&mut db, &roots, &build.docs).unwrap();
        let search = local::Documents::for_search(&helper, &token());
        let hits = super::query_index(&db, &roots, "deposit", &search, &token()).unwrap();
        assert_eq!(hits[0].excerpt.as_deref(), Some("Deposit returned 1200"));
        // The stored text only picks candidates: without a fresh read there is no match.
        assert!(query_index(&db, &roots, "deposit", &token()).unwrap().is_empty());
        std::fs::write(&pdf, "Deposit withheld 300").unwrap();
        let hits = super::query_index(&db, &roots, "deposit", &search, &token()).unwrap();
        assert_eq!(hits[0].excerpt.as_deref(), Some("Deposit withheld 300"));
    }

    #[test]
    fn invalid_replacement_cannot_erase_the_existing_index() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("notes.txt"), "needle").unwrap();
        let roots = vec![root(folder.path())];
        let mut build = rebuild(&roots, &token()).unwrap();
        let mut db = database();
        replace_index(&mut db, &roots, &build.docs).unwrap();
        build.docs[0].name = "different-path-name.txt".into();
        assert!(replace_index(&mut db, &roots, &build.docs).is_err());
        assert_eq!(
            query_index(&db, &roots, "needle", &token()).unwrap().len(),
            1
        );
    }
}
