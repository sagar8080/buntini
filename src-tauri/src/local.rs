use crate::types::*;
use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::{Duration, Instant},
};
use walkdir::{DirEntry, WalkDir};

pub const MAX_READ: u64 = 64 * 1024;
const MAX_ENTRIES: usize = 10_000;
const MAX_HITS: usize = 40;

pub fn authorize_root(path: &str) -> Result<String, String> {
    let p = Path::new(path);
    if !p.is_absolute() || !p.is_dir() {
        return Err("Select an existing absolute folder path.".into());
    }
    if fs::symlink_metadata(p)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Choose the original folder rather than a symbolic link.".into());
    }
    let canonical = p.canonicalize().map_err(|e| e.to_string())?;
    if canonical == Path::new("/") {
        return Err("Authorize a specific folder rather than the entire computer.".into());
    }
    Ok(canonical.to_string_lossy().into_owned())
}

fn visible(entry: &DirEntry) -> bool {
    entry.depth() == 0 || !entry.file_name().to_string_lossy().starts_with('.')
}

/// Open every path component relative to the authorized directory descriptor.
/// O_NOFOLLOW on all components prevents a symlink swap between validation and read.
#[cfg(unix)]
pub fn scoped_open(root: &Path, path: &Path) -> Result<File, String> {
    use std::{
        ffi::CString,
        os::fd::{AsRawFd, FromRawFd},
        os::unix::ffi::OsStrExt,
    };
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "File is outside authorized folders.")?;
    let parts = relative
        .components()
        .map(|c| match c {
            Component::Normal(s) if !s.as_bytes().starts_with(b".") => Ok(s),
            _ => Err("Hidden or traversal paths are not allowed."),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if parts.is_empty() {
        return Err("Expected a file.".into());
    }
    if !root.is_absolute() {
        return Err("Authorized roots must be absolute.".into());
    }
    let slash = CString::new("/").unwrap();
    let fd = unsafe {
        libc::open(
            slash.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err("Authorized folder is no longer accessible.".into());
    }
    let mut current = unsafe { File::from_raw_fd(fd) };
    // Protect the root's ancestors as well: an approved parent may have been
    // replaced with a symlink after permission was granted or the walk began.
    for component in root.components() {
        let Component::Normal(part) = component else {
            if component == Component::RootDir {
                continue;
            }
            return Err("Invalid authorized root.".into());
        };
        let name = CString::new(part.as_bytes()).map_err(|_| "Invalid root path.")?;
        let fd = unsafe {
            libc::openat(
                current.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err("Authorized folder ancestors must not be symbolic links.".into());
        }
        current = unsafe { File::from_raw_fd(fd) };
    }
    for (index, part) in parts.iter().enumerate() {
        let name = CString::new(part.as_bytes()).map_err(|_| "Invalid path.")?;
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | if index + 1 < parts.len() {
                libc::O_DIRECTORY
            } else {
                libc::O_NONBLOCK
            };
        let fd = unsafe { libc::openat(current.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err("File access denied; links and inaccessible files are skipped.".into());
        }
        current = unsafe { File::from_raw_fd(fd) };
    }
    if !current.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Only regular files may be read.".into());
    }
    Ok(current)
}
#[cfg(not(unix))]
pub fn scoped_open(root: &Path, path: &Path) -> Result<File, String> {
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    if !canonical.starts_with(root) {
        return Err("File outside authorized folder.".into());
    }
    std::fs::OpenOptions::new()
        .read(true)
        .open(canonical)
        .map_err(|e| e.to_string())
}

/// Plain text read as it is, up to `MAX_READ`.
const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "markdown", "csv", "tsv", "json", "yaml", "yml", "toml", "log", "rs", "py",
    "js", "ts", "tsx", "jsx", "html", "css",
];
/// Documents whose text the Apple helper extracts; the extension is the helper's kind.
/// An `.rtfd` package is a folder, so its `TXT.rtf` is read as RTF on the way through.
const DOCUMENT_EXTENSIONS: &[&str] = &["pdf", "docx", "doc", "rtf", "rtfd", "pages"];
/// The largest document handed to the helper, which refuses anything bigger itself.
pub const MAX_DOCUMENT_BYTES: u64 = 32 * 1024 * 1024;
/// The helper stops itself after 8 seconds; this is the backstop if it cannot.
const DOCUMENT_SECONDS: u64 = 10;
/// Escaped JSON for 64 KB of text fits well inside this.
const MAX_HELPER_OUTPUT: u64 = 1024 * 1024;
const CACHE_ENTRIES: usize = 1_024;
const CACHE_BYTES: usize = 16 * 1024 * 1024;

/// How a shared-folder file's content is read. Search, the file index and the memory
/// index's file source all go through this one list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Content {
    Text,
    Document(&'static str),
}

pub fn content_kind(path: &Path) -> Option<Content> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    if TEXT_EXTENSIONS.contains(&extension.as_str()) {
        return Some(Content::Text);
    }
    DOCUMENT_EXTENSIONS
        .iter()
        .find(|kind| **kind == extension)
        .map(|kind| Content::Document(kind))
}

/// Reads documents through the Apple helper for one walk. A file search may read only a
/// few new documents in a short time, so a folder of PDFs can't stall it; documents read
/// before come back from memory while they are unchanged. Without the helper, documents
/// are found by name only, as other files are.
pub struct Documents {
    helper: Option<PathBuf>,
    cancel: Arc<AtomicBool>,
    fresh: AtomicUsize,
    until: Option<Instant>,
    seconds: u64,
}

impl Documents {
    #[cfg(test)]
    pub fn none() -> Self {
        Self {
            helper: None,
            cancel: Arc::new(AtomicBool::new(false)),
            fresh: 0.into(),
            until: None,
            seconds: DOCUMENT_SECONDS,
        }
    }

    /// A search that has used all its fresh reads.
    #[cfg(test)]
    pub fn spent(helper: &Path, cancel: &Arc<AtomicBool>) -> Self {
        Self { fresh: 0.into(), ..Self::for_search(helper, cancel) }
    }

    pub fn for_search(helper: &Path, cancel: &Arc<AtomicBool>) -> Self {
        Self {
            helper: Some(helper.to_path_buf()),
            cancel: cancel.clone(),
            fresh: 24.into(),
            until: Some(Instant::now() + Duration::from_secs(4)),
            seconds: DOCUMENT_SECONDS,
        }
    }

    /// The user asked for the rebuild, so every document is read within the index's limits.
    pub fn for_index(helper: &Path, cancel: &Arc<AtomicBool>) -> Self {
        Self {
            fresh: usize::MAX.into(),
            until: None,
            ..Self::for_search(helper, cancel)
        }
    }

    /// Whether documents can be read at all (the helper is there).
    pub fn reads_documents(&self) -> bool {
        self.helper.as_deref().is_some_and(Path::is_file)
    }

    fn take_fresh(&self) -> bool {
        self.until.is_none_or(|until| Instant::now() < until)
            && self
                .fresh
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |left| left.checked_sub(1))
                .is_ok()
    }
}

type CacheKey = (PathBuf, u64, Option<std::time::SystemTime>, u64);

/// What the helper made of an unchanged document.
#[derive(Clone)]
enum Cached {
    Text(String),
    /// It refused the file (locked, unreadable, too much text): it has no text to give.
    Refused,
    /// It ran out of time. A search doesn't try it again; a rebuild the user asked for does.
    Stalled,
}

#[derive(Default)]
struct DocumentCache {
    texts: HashMap<CacheKey, Cached>,
    order: VecDeque<CacheKey>,
    bytes: usize,
}

/// Extracted text kept in memory only, keyed so any change to the file reads it again.
/// It is reached only after the file reopens inside a folder that is still shared.
static DOCUMENT_CACHE: LazyLock<Mutex<DocumentCache>> = LazyLock::new(Default::default);

fn cache_key(path: &Path, metadata: &fs::Metadata) -> CacheKey {
    #[cfg(unix)]
    let inode = std::os::unix::fs::MetadataExt::ino(metadata);
    #[cfg(not(unix))]
    let inode = 0;
    (path.to_path_buf(), metadata.len(), metadata.modified().ok(), inode)
}

fn cached(key: &CacheKey) -> Option<Cached> {
    DOCUMENT_CACHE.lock().ok()?.texts.get(key).cloned()
}

fn size(entry: &Cached) -> usize {
    match entry {
        Cached::Text(text) => text.len(),
        _ => 0,
    }
}

fn remember(key: CacheKey, entry: Cached) {
    let Ok(mut cache) = DOCUMENT_CACHE.lock() else {
        return;
    };
    let added = size(&entry);
    if let Some(old) = cache.texts.insert(key.clone(), entry) {
        cache.bytes -= size(&old);
        cache.order.retain(|existing| existing != &key);
    }
    cache.bytes += added;
    cache.order.push_back(key);
    while cache.order.len() > CACHE_ENTRIES || cache.bytes > CACHE_BYTES {
        let Some(oldest) = cache.order.pop_front() else {
            break;
        };
        if let Some(old) = cache.texts.remove(&oldest) {
            cache.bytes -= size(&old);
        }
    }
}

/// A file's searchable text: plain text up to `MAX_READ`, or a document's extracted text
/// cut to the same size. Unsupported, binary, invalid UTF-8, oversized or unreadable
/// files give no text and stay searchable by name. Either way the text is untrusted file
/// content, never instructions.
pub fn read_content(file: &mut File, path: &Path, documents: &Documents) -> String {
    read_text(file, path, documents).unwrap_or_default()
}

/// Like `read_content`, but None when a document's text wasn't read this time (the
/// search's budget ran out, the helper is missing, stalled or was stopped), which says
/// nothing about the file. Some("") means the file has no text to give.
pub fn read_text(file: &mut File, path: &Path, documents: &Documents) -> Option<String> {
    let Ok(metadata) = file.metadata() else {
        return Some(String::new());
    };
    match content_kind(path) {
        Some(Content::Text) if metadata.len() <= MAX_READ => {
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            if file.take(MAX_READ + 1).read_to_end(&mut bytes).is_err()
                || bytes.len() > MAX_READ as usize
                || bytes.contains(&0)
            {
                return Some(String::new());
            }
            Some(String::from_utf8(bytes).unwrap_or_default())
        }
        Some(Content::Document(kind)) if metadata.len() <= MAX_DOCUMENT_BYTES => {
            let helper = documents.helper.as_deref().filter(|helper| helper.is_file())?;
            let key = cache_key(path, &metadata);
            match cached(&key) {
                Some(Cached::Text(text)) => return Some(text),
                Some(Cached::Refused) => return Some(String::new()),
                // A slow document doesn't hold up every search; a rebuild tries it again.
                Some(Cached::Stalled) if documents.until.is_some() => return None,
                _ => {}
            }
            if !documents.take_fresh() {
                return None;
            }
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            if file.take(MAX_DOCUMENT_BYTES + 1).read_to_end(&mut bytes).is_err()
                || bytes.len() as u64 > MAX_DOCUMENT_BYTES
            {
                return None;
            }
            match extract(helper, kind, bytes, documents) {
                Ok(text) => {
                    remember(key, Cached::Text(text.clone()));
                    Some(text)
                }
                Err(Unread::Refused) => {
                    remember(key, Cached::Refused);
                    Some(String::new())
                }
                Err(Unread::Stalled) => {
                    remember(key, Cached::Stalled);
                    None
                }
                Err(Unread::Stopped) => None,
            }
        }
        _ => Some(String::new()),
    }
}

/// Why the helper gave no text.
enum Unread {
    /// It answered without text, or with more than a reply may hold.
    Refused,
    /// It used all its time.
    Stalled,
    /// It was cancelled, cut short by the search's deadline, or couldn't start.
    Stopped,
}

/// Runs `--extract-document` with the bytes on standard input, so the helper never opens
/// a path itself. Stops on cancel or after the timeout, and reads a bounded reply.
fn extract(helper: &Path, kind: &str, bytes: Vec<u8>, documents: &Documents) -> Result<String, Unread> {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    // A search's deadline bounds the read as well as when it may start.
    let full = Duration::from_secs(documents.seconds);
    let limit = documents
        .until
        .map(|until| full.min(until.saturating_duration_since(Instant::now()).max(Duration::from_millis(500))))
        .unwrap_or(full);
    let mut child = Command::new(helper)
        .args(["--extract-document", kind])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Unread::Stopped)?;
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Unread::Stopped);
    };
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&bytes);
    });
    let reader = std::thread::spawn(move || {
        let mut reply = Vec::new();
        let _ = stdout.take(MAX_HELPER_OUTPUT + 1).read_to_end(&mut reply);
        reply
    });
    let started = Instant::now();
    let finished = loop {
        match child.try_wait() {
            Ok(Some(_)) => break true,
            Ok(None)
                if started.elapsed() < limit
                    && !documents.cancel.load(Ordering::Relaxed) =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    // A stopped helper's pipes close with it; the threads are not waited on in case
    // anything it started still holds them.
    if !finished {
        let stalled = limit == full && started.elapsed() >= full && !documents.cancel.load(Ordering::Relaxed);
        return Err(if stalled { Unread::Stalled } else { Unread::Stopped });
    }
    let _ = writer.join();
    let reply = reader.join().map_err(|_| Unread::Stopped)?;
    if reply.len() as u64 > MAX_HELPER_OUTPUT {
        return Err(Unread::Refused);
    }
    document_text(&reply).ok_or(Unread::Refused)
}

/// The helper's one-line reply as text, or nothing when it refused the file.
fn document_text(reply: &[u8]) -> Option<String> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Reply {
        text: String,
        #[allow(dead_code)]
        truncated: bool,
    }
    let line = reply.split(|byte| *byte == b'\n').next()?;
    let reply: Reply = serde_json::from_slice(line).ok()?;
    let mut text = reply.text.replace('\0', "");
    if text.len() > MAX_READ as usize {
        let mut end = MAX_READ as usize;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    Some(text)
}

/// A conservative regular-plural alternative, never a replacement for the
/// user's query. Identifiers, acronyms and common non-plural endings stay
/// literal; callers use alternatives only when no literal result survives.
pub(crate) fn singular_search_term(term: &str) -> Option<String> {
    if term.len() < 4
        || !term.chars().all(|ch| ch.is_ascii_alphabetic())
        || term.chars().all(|ch| ch.is_ascii_uppercase())
        || term.chars().skip(1).any(|ch| ch.is_ascii_uppercase())
    {
        return None;
    }
    let lower = term.to_ascii_lowercase();
    if !lower.ends_with('s')
        || ["ss", "us", "is", "as", "ous", "ness"]
            .iter()
            .any(|suffix| lower.ends_with(suffix))
    {
        return None;
    }
    if lower.ends_with("ies") && lower.len() > 4 {
        return Some(format!("{}y", &lower[..lower.len() - 3]));
    }
    let drop = if ["ches", "shes", "xes", "zes"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
    {
        2
    } else {
        1
    };
    Some(lower[..lower.len() - drop].to_string())
}

pub fn search(
    roots: &[String],
    query: &str,
    documents: &Documents,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<(String, FileHit, String)>, String> {
    search_group(roots, query, documents, cancel).map(|group| {
        group
            .hits
            .into_iter()
            .map(|(root, hit, content)| (root, hit, content.unwrap_or_default()))
            .collect()
    })
}

/// A search backend returns one homogeneous group: literal matches when any
/// survive, otherwise plural alternatives. Preserve that distinction when
/// combining multiple bounded searches so fallback rows cannot crowd out exact
/// hits discovered by another backend.
pub(crate) struct MatchGroup<T> {
    pub hits: Vec<T>,
    pub literal: bool,
}

pub(crate) fn prefer_literal<T>(literal: Vec<T>, fallback: Vec<T>) -> MatchGroup<T> {
    if literal.is_empty() {
        MatchGroup {
            hits: fallback,
            literal: false,
        }
    } else {
        MatchGroup {
            hits: literal,
            literal: true,
        }
    }
}

/// A word that asks rather than names: no file is found by it.
pub(crate) fn question_word(word: &str) -> bool {
    const QUESTION: [&str; 28] = [
        "when", "what", "where", "which", "who", "how", "why", "does", "do", "did", "is", "are",
        "was", "the", "a", "an", "my", "i", "me", "of", "in", "on", "for", "about", "will", "can",
        "it", "to",
    ];
    QUESTION.contains(&word.to_lowercase().as_str())
}

/// The words a file must contain. A question ("when does the lease renew?") is searched
/// by its content words; its question words and punctuation are in no file. A query made
/// only of such words is searched as written.
fn search_terms(query: &str) -> Vec<&str> {
    let all: Vec<&str> = query.split_whitespace().collect();
    let content: Vec<&str> = all
        .iter()
        .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|word| !word.is_empty() && !question_word(word))
        .collect();
    if content.is_empty() { all } else { content }
}

pub(crate) fn search_group(
    roots: &[String],
    query: &str,
    documents: &Documents,
    cancel: &Arc<AtomicBool>,
) -> Result<MatchGroup<(String, FileHit, Option<String>)>, String> {
    let query = query.trim();
    if query.is_empty() || query.len() > 512 {
        return Err("Enter a file name or a few search words (up to 512 characters).".into());
    }
    let terms = search_terms(query);
    let words: Vec<String> = terms.iter().map(|term| term.to_lowercase()).collect();
    let alternatives = terms
        .iter()
        .map(|term| singular_search_term(term))
        .collect::<Vec<_>>();
    let mut result = Vec::new();
    let mut fallback = Vec::new();
    let mut scanned = 0;
    for root_str in roots {
        let root = PathBuf::from(root_str);
        if root.canonicalize().ok().as_ref() != Some(&root) {
            continue;
        }
        for item in WalkDir::new(&root)
            .follow_links(false)
            .max_depth(20)
            .into_iter()
            .filter_entry(visible)
        {
            if cancel.load(Ordering::Relaxed) {
                return Err("Task cancelled.".into());
            }
            scanned += 1;
            if scanned > MAX_ENTRIES || result.len() >= MAX_HITS {
                return Ok(prefer_literal(result, fallback));
            }
            let entry = match item {
                Ok(e) => e,
                Err(_) => continue,
            };
            if !entry.file_type().is_file() || entry.file_type().is_symlink() {
                continue;
            }
            let mut file = match scoped_open(&root, entry.path()) {
                Ok(f) => f,
                Err(_) => continue,
            };
            let metadata = match file.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let name_lower = name.to_lowercase();
            let read = read_text(&mut file, entry.path(), documents);
            let content = read.clone().unwrap_or_default();
            let lower = content.to_lowercase();
            let exact = words
                .iter()
                .all(|w| name_lower.contains(w) || lower.contains(w));
            let relaxed = result.is_empty()
                && fallback.len() < MAX_HITS
                && words.iter().zip(&alternatives).all(|(word, alternative)| {
                    name_lower.contains(word)
                        || lower.contains(word)
                        || alternative.as_ref().is_some_and(|singular| {
                            name_lower.contains(singular) || lower.contains(singular)
                        })
                });
            if !exact && !relaxed {
                continue;
            }
            let excerpt_words = words
                .iter()
                .zip(&alternatives)
                .map(|(word, alternative)| {
                    if exact {
                        word
                    } else {
                        alternative.as_ref().unwrap_or(word)
                    }
                })
                .collect::<Vec<_>>();
            let excerpt = if content.is_empty() {
                None
            } else {
                let lines = content
                    .lines()
                    .filter(|line| {
                        excerpt_words
                            .iter()
                            .any(|w| line.to_lowercase().contains(w.as_str()))
                    })
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" ");
                let lines = if lines.is_empty() {
                    content
                        .lines()
                        .filter(|line| !line.trim().is_empty())
                        .take(5)
                        .collect::<Vec<_>>()
                        .join(" ")
                } else {
                    lines
                };
                Some(lines.chars().take(600).collect())
            };
            let modified_at = metadata
                .modified()
                .ok()
                .map(chrono::DateTime::<chrono::Utc>::from)
                .map(|d| d.to_rfc3339())
                .unwrap_or_default();
            let hit = (
                root_str.clone(),
                FileHit {
                    name,
                    path: entry.path().to_string_lossy().into_owned(),
                    size: metadata.len(),
                    modified_at,
                    excerpt,
                },
                read,
            );
            if exact {
                fallback.clear();
                result.push(hit);
            } else {
                fallback.push(hit);
            }
        }
    }
    Ok(prefer_literal(result, fallback))
}

pub fn snapshot() -> Result<SystemSnapshot, String> {
    use sysinfo::{Disks, ProcessesToUpdate, System};
    let unreadable = |_| "macOS didn't report this Mac's system information. Try again.".to_string();
    let mut sys = System::new_all().map_err(unreadable)?;
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let mut processes = sys
        .processes()
        .values()
        .map(|p| ProcessInfo {
            name: p.name().to_string_lossy().into_owned(),
            cpu_percent: p.cpu_usage(),
            memory_bytes: p.memory(),
        })
        .collect::<Vec<_>>();
    processes.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
    processes.truncate(8);
    let disks = Disks::new_with_refreshed_list()
        .map_err(unreadable)?
        .iter()
        .map(|d| DiskInfo {
            name: d.name().to_string_lossy().into_owned(),
            total_bytes: d.total_space(),
            available_bytes: d.available_space(),
        })
        .collect();
    Ok(SystemSnapshot {
        cpu_percent: sys.global_cpu_usage(),
        memory_used: sys.used_memory(),
        memory_total: sys.total_memory(),
        uptime_seconds: System::uptime().map_err(unreadable)?,
        os: format!(
            "{} {}",
            System::name().unwrap_or_default(),
            System::os_version().unwrap_or_default()
        ),
        disks,
        processes,
        captured_at: now(),
        security: security_snapshot(),
    })
}
fn fixed_status(program: &str, args: &[&str]) -> String {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return "Unavailable on this Mac".into(),
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    child
                        .wait_with_output()
                        .ok()
                        .map(|o| {
                            String::from_utf8_lossy(&o.stdout)
                                .chars()
                                .take(1000)
                                .collect::<String>()
                                .trim()
                                .to_string()
                        })
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "Status unavailable".into())
                } else {
                    "Status unavailable (no changes were made)".into()
                };
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return "Status unavailable".into();
            }
            _ => {}
        }
        if start.elapsed() > Duration::from_secs(2) {
            let _ = child.kill();
            let _ = child.wait();
            return "Status check timed out".into();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn security_snapshot() -> SecuritySnapshot {
    SecuritySnapshot{file_vault:fixed_status("/usr/bin/fdesetup",&["status"]),gatekeeper:fixed_status("/usr/sbin/spctl",&["--status"]),system_integrity:fixed_status("/usr/bin/csrutil",&["status"]),os_update_status:"Not checked. Open System Settings → General → Software Update to check available updates.".into()}
}
pub fn system_answer(s: &SystemSnapshot) -> String {
    let gb = 1024.0 * 1024.0 * 1024.0;
    format!("Your Mac is using {:.0}% CPU and {:.1} GB of {:.1} GB memory. Uptime is {} hours. {}\n\nFileVault: {}\nGatekeeper: {}\nSystem Integrity Protection: {}\nOS updates: {}",s.cpu_percent,s.memory_used as f64/gb,s.memory_total as f64/gb,s.uptime_seconds/3600,
        s.disks.iter().map(|d|format!("{} has {:.1} GB free of {:.1} GB.",d.name,d.available_bytes as f64/gb,d.total_bytes as f64/gb)).collect::<Vec<_>>().join(" "),s.security.file_vault,s.security.gatekeeper,s.security.system_integrity,s.security.os_update_status)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Plain-text searches; document tests pass their own reader.
    fn search(
        roots: &[String],
        query: &str,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Vec<(String, FileHit, String)>, String> {
        super::search(roots, query, &Documents::none(), cancel)
    }
    #[cfg(unix)]
    #[test]
    fn symlink_escape_and_hidden_files_are_denied() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        fs::write(dir.path().join("visible.txt"), "needle").unwrap();
        fs::write(dir.path().join(".hidden.txt"), "needle").unwrap();
        symlink(outside.path(), dir.path().join("escape")).unwrap();
        symlink(
            outside.path().join("secret.txt"),
            dir.path().join("link.txt"),
        )
        .unwrap();
        assert!(scoped_open(dir.path(), &dir.path().join("escape/secret.txt")).is_err());
        assert!(scoped_open(dir.path(), &dir.path().join("link.txt")).is_err());
        assert!(scoped_open(dir.path(), &dir.path().join(".hidden.txt")).is_err());
        let canonical = dir.path().canonicalize().unwrap();
        let hits = search(
            &[canonical.to_string_lossy().into_owned()],
            "needle",
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1.name, "visible.txt");
    }
    #[test]
    fn a_question_is_searched_by_its_content_words() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("lease-notes.txt"), "The lease renews on 14 March.").unwrap();
        fs::write(dir.path().join("other.txt"), "Nothing about it here.").unwrap();
        let root = dir.path().canonicalize().unwrap().to_string_lossy().into_owned();
        let cancel = Arc::new(AtomicBool::new(false));
        let found = |query: &str| search(&[root.clone()], query, &cancel).unwrap().into_iter().map(|hit| hit.1.name).collect::<Vec<_>>();
        assert_eq!(found("when does the lease renew?"), ["lease-notes.txt"]);
        assert_eq!(found("lease"), ["lease-notes.txt"]);
        assert_eq!(search_terms("what is it?"), ["what", "is", "it?"], "only question words: searched as written");
    }
    #[test]
    fn sibling_prefix_is_not_an_authorized_child() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path().join("approved");
        fs::create_dir(&r).unwrap();
        let outside = d.path().join("approved-elsewhere");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "x").unwrap();
        assert!(scoped_open(&r, &outside.join("secret.txt")).is_err());
    }
    #[test]
    fn large_content_is_not_read_and_cancellation_is_observed() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join("large.txt"),
            vec![b'x'; MAX_READ as usize + 1],
        )
        .unwrap();
        let root = d
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let hits = search(&[root.clone()], "large", &Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].2.is_empty());
        assert!(search(&[root], "large", &Arc::new(AtomicBool::new(true))).is_err());
    }

    #[test]
    fn plural_fallback_reads_real_content_and_retains_identifier_spelling() {
        let folder = tempfile::tempdir().unwrap();
        fs::write(
            folder.path().join("vendor-offer.txt"),
            "ACME-6279 quote: Cedar costs 680 including assembly.",
        )
        .unwrap();
        let roots = vec![folder
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned()];
        let hits = search(
            &roots,
            "ACME-6279 Quotes",
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0]
            .1
            .excerpt
            .as_deref()
            .unwrap()
            .contains("680 including assembly"));
        assert!(search(
            &roots,
            "ACME-6280 Quotes",
            &Arc::new(AtomicBool::new(false))
        )
        .unwrap()
        .is_empty());

        for name in ["Atlas", "Polaris", "BUS", "AcmeOps", "ACME-6279s"] {
            assert!(singular_search_term(name).is_none(), "{name}");
            fs::write(folder.path().join("proper-name.txt"), name).unwrap();
            let hits = search(&roots, name, &Arc::new(AtomicBool::new(false))).unwrap();
            assert_eq!(hits.len(), 1, "{name}");
            assert_eq!(hits[0].1.name, "proper-name.txt");
        }
    }

    #[test]
    fn exact_hits_override_a_full_fallback_buffer_and_both_are_bounded() {
        let fallback_folder = tempfile::tempdir().unwrap();
        let exact_folder = tempfile::tempdir().unwrap();
        for index in 0..MAX_HITS + 5 {
            fs::write(
                fallback_folder.path().join(format!("offer-{index}.txt")),
                "vendor quote total 680",
            )
            .unwrap();
        }
        fs::write(
            exact_folder.path().join("literal.txt"),
            "vendor quotes total 745",
        )
        .unwrap();
        let root = |path: &Path| path.canonicalize().unwrap().to_string_lossy().into_owned();
        let first_root = root(fallback_folder.path());
        let cancel = Arc::new(AtomicBool::new(false));
        assert_eq!(
            search(&[first_root.clone()], "quotes", &cancel)
                .unwrap()
                .len(),
            MAX_HITS
        );
        let hits = search(&[first_root, root(exact_folder.path())], "quotes", &cancel).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1.name, "literal.txt");
        // A proper name with the same regular suffix also keeps its literal
        // match; the relaxed candidate cannot replace the requested spelling.
        fs::write(fallback_folder.path().join("name.txt"), "Charle").unwrap();
        fs::write(exact_folder.path().join("name.txt"), "Charles").unwrap();
        let hits = search(
            &[root(fallback_folder.path()), root(exact_folder.path())],
            "Charles",
            &cancel,
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].2, "Charles");
    }

    #[cfg(unix)]
    #[test]
    fn plural_fallback_cannot_read_hidden_symlink_or_revoked_scopes() {
        use std::os::unix::fs::symlink;
        let folder = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(folder.path().join(".private.txt"), "vendor quote hidden").unwrap();
        fs::write(outside.path().join("private.txt"), "vendor quote outside").unwrap();
        symlink(
            outside.path().join("private.txt"),
            folder.path().join("link.txt"),
        )
        .unwrap();
        let roots = vec![folder
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned()];
        let cancel = Arc::new(AtomicBool::new(false));
        assert!(search(&roots, "quotes", &cancel).unwrap().is_empty());
        assert!(search(&[], "quotes", &cancel).unwrap().is_empty());
    }
    /// The sysinfo paths behind "How is my Mac doing?" on the Mac running the tests.
    #[cfg(target_os = "macos")]
    #[test]
    fn system_snapshot_reads_this_mac() {
        let snapshot = snapshot().unwrap();
        assert!(snapshot.memory_total > 0 && snapshot.memory_used <= snapshot.memory_total);
        assert!(snapshot.uptime_seconds > 0);
        assert!(snapshot.os.contains("macOS") || snapshot.os.contains("Darwin"), "{}", snapshot.os);
        assert!(!snapshot.disks.is_empty());
        assert!(snapshot.disks.iter().all(|disk| disk.available_bytes <= disk.total_bytes));
        assert!(!snapshot.processes.is_empty() && snapshot.processes.len() <= 8);
        assert!(snapshot.processes.windows(2).all(|pair| pair[0].cpu_percent >= pair[1].cpu_percent));
        assert!(system_answer(&snapshot).starts_with("Your Mac is using"));
    }


    #[test]
    fn one_extension_list_decides_text_documents_and_name_only_files() {
        for (name, kind) in [
            ("notes.txt", Some(Content::Text)),
            ("README.MD", Some(Content::Text)),
            ("Lease.PDF", Some(Content::Document("pdf"))),
            ("offer.docx", Some(Content::Document("docx"))),
            ("old.doc", Some(Content::Document("doc"))),
            ("letter.rtf", Some(Content::Document("rtf"))),
            ("flat.rtfd", Some(Content::Document("rtfd"))),
            ("plan.pages", Some(Content::Document("pages"))),
            ("page.webarchive", None),
            ("photo.jpg", None),
            ("Makefile", None),
        ] {
            assert_eq!(content_kind(Path::new(name)), kind, "{name}");
        }
    }

    fn folder_root(path: &Path) -> String {
        path.canonicalize().unwrap().to_string_lossy().into_owned()
    }

    /// A stand-in for the Apple helper: a shell script with the same command line.
    #[cfg(unix)]
    fn fake_helper(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("buntini-fm");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn documents_are_read_from_piped_bytes_and_found_by_their_text() {
        let tools = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let runs = tools.path().join("runs");
        // The helper gets only the kind and the bytes, never the file's path.
        let helper = fake_helper(
            tools.path(),
            &format!(
                r#"echo run >> '{}'
[ "$#" = 2 ] && [ "$1" = --extract-document ] && [ "$2" = pdf ] || exit 2
[ "$(wc -c | tr -d ' ')" = 11 ] || exit 3
printf '%s\n' '{{"text":"Lease renewal total 680","truncated":false}}'"#,
                runs.display()
            ),
        );
        let pdf = folder.path().join("scan.pdf");
        fs::write(&pdf, "%PDF-binary").unwrap();
        let roots = vec![folder_root(folder.path())];
        let cancel = Arc::new(AtomicBool::new(false));
        let hits = super::search(&roots, "renewal", &Documents::for_search(&helper, &cancel), &cancel).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1.name, "scan.pdf");
        assert!(hits[0].1.excerpt.as_deref().unwrap().contains("680"));
        // Unchanged documents come from memory; a changed one is read again.
        super::search(&roots, "renewal", &Documents::for_search(&helper, &cancel), &cancel).unwrap();
        assert_eq!(fs::read_to_string(&runs).unwrap().lines().count(), 1);
        fs::write(&pdf, "%PDF-binarY").unwrap();
        let later = std::time::SystemTime::now() + Duration::from_secs(5);
        File::options().write(true).open(&pdf).unwrap().set_modified(later).unwrap();
        super::search(&roots, "renewal", &Documents::for_search(&helper, &cancel), &cancel).unwrap();
        assert_eq!(fs::read_to_string(&runs).unwrap().lines().count(), 2);
        // Without the helper the document is still found by name, never by content.
        assert!(search(&roots, "renewal", &cancel).unwrap().is_empty());
        assert_eq!(search(&roots, "scan", &cancel).unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn refused_hung_oversized_and_over_budget_documents_give_no_text() {
        let tools = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let read = |name: &str, documents: Documents| {
            let path = folder.path().join(name);
            read_content(&mut File::open(&path).unwrap(), &path, &documents)
        };
        fs::write(folder.path().join("locked.pdf"), "x").unwrap();
        let refusing = fake_helper(tools.path(), r#"cat >/dev/null; printf '%s\n' '{"error":"locked"}'"#);
        assert_eq!(read("locked.pdf", Documents::for_search(&refusing, &cancel)), "");

        let slow = tempfile::tempdir().unwrap();
        let hung = fake_helper(slow.path(), "sleep 30 && echo late");
        fs::write(folder.path().join("hang.docx"), "x").unwrap();
        let started = Instant::now();
        let documents = Documents { seconds: 1, ..Documents::for_search(&hung, &cancel) };
        assert_eq!(read("hang.docx", documents), "");
        assert!(started.elapsed() < Duration::from_secs(5));
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = stopped.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::Relaxed);
        });
        fs::write(folder.path().join("cancel.doc"), "x").unwrap();
        let started = Instant::now();
        assert_eq!(read("cancel.doc", Documents::for_search(&hung, &stopped)), "");
        assert!(started.elapsed() < Duration::from_secs(5));

        let noisy = tempfile::tempdir().unwrap();
        let flood = fake_helper(noisy.path(), "cat >/dev/null; head -c 3000000 /dev/zero | tr '\\0' a");
        fs::write(folder.path().join("flood.rtf"), "x").unwrap();
        assert_eq!(read("flood.rtf", Documents::for_search(&flood, &cancel)), "");

        let long = tempfile::tempdir().unwrap();
        let wordy = fake_helper(
            long.path(),
            "cat >/dev/null; printf '{\"text\":\"'; head -c 70000 /dev/zero | tr '\\0' a; printf '\",\"truncated\":true}\\n'",
        );
        fs::write(folder.path().join("long.pages"), "x").unwrap();
        let text = read("long.pages", Documents::for_search(&wordy, &cancel));
        assert_eq!(text.len(), MAX_READ as usize);

        let counted = tempfile::tempdir().unwrap();
        let runs = counted.path().join("runs");
        let counting = fake_helper(
            counted.path(),
            &format!(r#"echo run >> '{}'; cat >/dev/null; printf '%s\n' '{{"text":"ok","truncated":false}}'"#, runs.display()),
        );
        let big = folder.path().join("big.pdf");
        File::create(&big).unwrap().set_len(MAX_DOCUMENT_BYTES + 1).unwrap();
        assert_eq!(read("big.pdf", Documents::for_search(&counting, &cancel)), "");
        assert!(!runs.exists(), "an oversized document is never handed to the helper");
        fs::write(folder.path().join("one.pdf"), "1").unwrap();
        fs::write(folder.path().join("two.pdf"), "2").unwrap();
        let documents = Documents { fresh: 1.into(), ..Documents::for_search(&counting, &cancel) };
        let path = folder.path().join("one.pdf");
        assert_eq!(read_content(&mut File::open(&path).unwrap(), &path, &documents), "ok");
        let path = folder.path().join("two.pdf");
        assert_eq!(read_content(&mut File::open(&path).unwrap(), &path, &documents), "");
        assert_eq!(fs::read_to_string(&runs).unwrap().lines().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_document_the_helper_refused_or_stalled_on_is_not_tried_again_by_search() {
        let tools = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let runs = tools.path().join("runs");
        let refusing = fake_helper(
            tools.path(),
            &format!(r#"echo run >> '{}'; cat >/dev/null; printf '%s\n' '{{"error":"locked"}}'"#, runs.display()),
        );
        let path = folder.path().join("locked.pdf");
        fs::write(&path, "x").unwrap();
        let read = |documents: &Documents| read_text(&mut File::open(&path).unwrap(), &path, documents);
        assert_eq!(read(&Documents::for_search(&refusing, &cancel)).as_deref(), Some(""), "it has no text");
        assert_eq!(read(&Documents::for_search(&refusing, &cancel)).as_deref(), Some(""));
        assert_eq!(fs::read_to_string(&runs).unwrap().lines().count(), 1, "refused once, remembered");

        let slow = tempfile::tempdir().unwrap();
        let stalled_runs = slow.path().join("runs");
        // Long enough for a busy test machine to start the helper before it is stopped.
        let hung = fake_helper(slow.path(), &format!("echo run >> '{}'; sleep 30", stalled_runs.display()));
        let stalled = folder.path().join("slow.docx");
        fs::write(&stalled, "x").unwrap();
        let read = |documents: &Documents| read_text(&mut File::open(&stalled).unwrap(), &stalled, documents);
        assert_eq!(read(&Documents { seconds: 3, ..Documents::for_search(&hung, &cancel) }), None, "not read, which says nothing about the file");
        assert_eq!(read(&Documents { seconds: 3, ..Documents::for_search(&hung, &cancel) }), None);
        assert_eq!(fs::read_to_string(&stalled_runs).unwrap().lines().count(), 1, "a search doesn't wait on it again");
        assert_eq!(read(&Documents { seconds: 3, ..Documents::for_index(&hung, &cancel) }), None);
        assert_eq!(fs::read_to_string(&stalled_runs).unwrap().lines().count(), 2, "a rebuild tries again");

        // The search's deadline bounds a read that has started, not only when it starts.
        let late = folder.path().join("late.pages");
        fs::write(&late, "x").unwrap();
        let documents = Documents { until: Some(Instant::now() + Duration::from_millis(600)), ..Documents::for_search(&hung, &cancel) };
        let started = Instant::now();
        assert_eq!(read_text(&mut File::open(&late).unwrap(), &late, &documents), None);
        assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
        // Out of budget: not read.
        let documents = Documents { fresh: 0.into(), ..Documents::for_search(&refusing, &cancel) };
        let other = folder.path().join("other.pdf");
        fs::write(&other, "y").unwrap();
        assert_eq!(read_text(&mut File::open(&other).unwrap(), &other, &documents), None);
    }

    /// Runs the real Apple helper when this Mac built it (Apple silicon, macOS 26 SDK).
    #[test]
    fn the_apple_helper_reads_an_rtf_document() {
        let helper = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../native/buntini-fm");
        if !helper.is_file() {
            return;
        }
        let folder = tempfile::tempdir().unwrap();
        fs::write(
            folder.path().join("letter.rtf"),
            r#"{\rtf1\ansi Finch lease renewal costs 745 {\field{\*\fldinst HYPERLINK "https://tracker.invalid/"}{\fldrslt here}}.}"#,
        )
        .unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let hits = super::search(
            &[folder_root(folder.path())],
            "renewal",
            &Documents::for_search(&helper, &cancel),
            &cancel,
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].2.contains("Finch lease renewal costs 745"), "{}", hits[0].2);
        assert!(!hits[0].2.contains("tracker.invalid"));
    }
    #[test]
    #[ignore = "Run explicitly on the target Mac to record current tool latency"]
    fn measure_system_snapshot_latency() {
        let started = std::time::Instant::now();
        let snapshot = snapshot().unwrap();
        println!(
            "system_snapshot_ms={} memory_total={} includes_cpu_sample_ms={}",
            started.elapsed().as_millis(),
            snapshot.memory_total,
            sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.as_millis()
        );
        assert!(snapshot.memory_total > 0);
    }
}
