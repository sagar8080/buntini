use crate::types::*;
use std::{
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
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

fn text_extension(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase()
            .as_str(),
        "txt"
            | "md"
            | "markdown"
            | "csv"
            | "tsv"
            | "json"
            | "yaml"
            | "yml"
            | "toml"
            | "log"
            | "rs"
            | "py"
            | "js"
            | "ts"
            | "tsx"
            | "jsx"
            | "html"
            | "css"
    )
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
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<(String, FileHit, String)>, String> {
    search_group(roots, query, cancel).map(|group| group.hits)
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

pub(crate) fn search_group(
    roots: &[String],
    query: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<MatchGroup<(String, FileHit, String)>, String> {
    let query = query.trim();
    if query.is_empty() || query.len() > 512 {
        return Err("Enter a file name or a few search words (up to 512 characters).".into());
    }
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let alternatives = query
        .split_whitespace()
        .map(singular_search_term)
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
            let mut content = String::new();
            if text_extension(entry.path()) && metadata.len() <= MAX_READ {
                let mut bytes = Vec::new();
                if file.by_ref().take(MAX_READ).read_to_end(&mut bytes).is_ok()
                    && !bytes.contains(&0)
                {
                    content = String::from_utf8(bytes).unwrap_or_default();
                }
            }
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
                content,
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
