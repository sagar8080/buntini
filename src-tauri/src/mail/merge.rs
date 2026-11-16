//! Splitting a PDF for a mail merge through `buntini-fm --split-pdf`. The helper gets the
//! bytes on standard input and confines itself before parsing; it answers one JSON line per
//! page with the page's text and the page as its own PDF. Buntini writes the pages it attaches.
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The largest PDF read for a merge, as for documents.
pub const MAX_PDF: u64 = 32 * 1024 * 1024;
/// The most the helper may answer: every page as base64, with room for repeated fonts.
const MAX_ANSWER: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub text: String,
    pub pdf: Vec<u8>,
}

fn failure(code: &str) -> String {
    match code {
        "locked" => "That PDF needs a password.".into(),
        "too_large" => "That PDF is larger than 32 MB.".into(),
        "too_many_pages" => "That PDF has more than 200 pages.".into(),
        "empty" => "That PDF has no pages.".into(),
        "timed_out" => "Splitting that PDF took too long.".into(),
        _ => "That PDF couldn’t be read.".into(),
    }
}

/// The helper's answer, page by page.
pub fn parse(answer: &str) -> Result<Vec<Page>, String> {
    let mut pages = Vec::new();
    for line in answer.lines().filter(|line| !line.trim().is_empty()) {
        let value: serde_json::Value = serde_json::from_str(line).map_err(|_| failure(""))?;
        if let Some(code) = value["error"].as_str() {
            return Err(failure(code));
        }
        if value["page"].as_u64() != Some(pages.len() as u64 + 1) {
            return Err(failure(""));
        }
        let pdf = base64::engine::general_purpose::STANDARD.decode(value["pdf"].as_str().unwrap_or("")).map_err(|_| failure(""))?;
        if !pdf.starts_with(b"%PDF") {
            return Err(failure(""));
        }
        pages.push(Page { text: value["text"].as_str().unwrap_or("").to_string(), pdf });
    }
    if pages.is_empty() {
        return Err(failure("empty"));
    }
    Ok(pages)
}

/// Each page of the PDF at `path`.
pub async fn split(helper: &Path, path: &Path) -> Result<Vec<Page>, String> {
    let size = tokio::fs::metadata(path).await.map_err(|_| "That PDF couldn’t be opened.")?.len();
    if size > MAX_PDF {
        return Err(failure("too_large"));
    }
    let bytes = tokio::fs::read(path).await.map_err(|_| "That PDF couldn’t be opened.")?;
    if !helper.is_file() {
        return Err("Splitting a PDF needs Buntini’s Apple helper, which isn’t on this Mac.".into());
    }
    let mut child = tokio::process::Command::new(helper)
        .arg("--split-pdf")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "Buntini’s Apple helper didn’t start.")?;
    let mut stdin = child.stdin.take().ok_or("Buntini’s Apple helper didn’t start.")?;
    let stdout = child.stdout.take().ok_or("Buntini’s Apple helper didn’t start.")?;
    let work = async {
        let writer = async move {
            let _ = stdin.write_all(&bytes).await;
            drop(stdin);
        };
        let reader = async {
            let mut answer = Vec::new();
            stdout.take(MAX_ANSWER + 1).read_to_end(&mut answer).await.map(|_| answer)
        };
        let ((), answer) = tokio::join!(writer, reader);
        let answer = answer.map_err(|_| failure(""))?;
        if answer.len() as u64 > MAX_ANSWER {
            return Err(failure("too_large"));
        }
        let _ = child.wait().await;
        parse(&String::from_utf8_lossy(&answer))
    };
    tokio::time::timeout(Duration::from_secs(40), work).await.map_err(|_| failure("timed_out"))?
}

/// A file name safe for Mail and Finder: no slashes, colons or control characters, short.
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || ['/', ':', '\\'].contains(&c) { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned = cleaned.trim_start_matches('.').trim();
    let cut: String = cleaned.chars().take(80).collect();
    if cut.is_empty() { "Page".into() } else { cut }
}

/// "payslips - Ana Ruiz.pdf" in `dir`.
pub fn page_file(dir: &Path, pdf_name: &str, person: &str) -> PathBuf {
    let stem = Path::new(pdf_name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    dir.join(format!("{} - {}.pdf", safe_name(&stem), safe_name(person)))
}

/// Removes folders under `dir` last changed more than a day ago: pages and attachments
/// copied for drafts that Mail has long since saved.
pub fn tidy(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let day = std::time::SystemTime::now() - Duration::from_secs(24 * 3600);
    for entry in entries.flatten() {
        let old = entry.metadata().and_then(|m| m.modified()).is_ok_and(|at| at < day);
        if old && entry.file_type().is_ok_and(|t| t.is_dir()) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_read_in_order_and_errors_say_why() {
        let pdf = base64::engine::general_purpose::STANDARD.encode(b"%PDF-1.7 one page");
        let answer = format!("{{\"page\":1,\"text\":\"Ana Ruiz\",\"pdf\":\"{pdf}\"}}\n{{\"page\":2,\"text\":\"Bo\",\"pdf\":\"{pdf}\"}}\n");
        let pages = parse(&answer).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].text, "Ana Ruiz");
        assert!(pages[1].pdf.starts_with(b"%PDF"));
        assert_eq!(parse("{\"error\":\"locked\"}").unwrap_err(), "That PDF needs a password.");
        // Out of order, not a PDF, or nothing at all.
        assert!(parse(&format!("{{\"page\":2,\"text\":\"\",\"pdf\":\"{pdf}\"}}")).is_err());
        assert!(parse("{\"page\":1,\"text\":\"\",\"pdf\":\"aGVsbG8=\"}").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn page_files_are_named_for_the_person() {
        let dir = Path::new("/tmp/merge");
        assert_eq!(page_file(dir, "payslips.pdf", "Ana Ruiz"), dir.join("payslips - Ana Ruiz.pdf"));
        assert_eq!(page_file(dir, "../x/y.pdf", "a/b:c\n.d"), dir.join("y - a b c .d.pdf"));
        assert_eq!(safe_name("...hidden"), "hidden");
        assert_eq!(safe_name("\n"), "Page");
    }

    /// Runs the real helper on a PDF made here: build it first with native/build-fm.sh.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "uses native/buntini-fm; build it first"]
    async fn live_the_helper_splits_a_pdf_page_by_page() {
        let helper = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../native/buntini-fm");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("three.pdf");
        std::fs::write(&path, tests_pdf()).unwrap();
        let pages = split(&helper, &path).await.unwrap();
        assert_eq!(pages.len(), 3);
        assert!(pages[1].text.contains("Bo Ruiz"), "{:?}", pages[1].text);
        assert!(pages.iter().all(|p| p.pdf.starts_with(b"%PDF")));
        let missing = split(&helper, &dir.path().join("none.pdf")).await.unwrap_err();
        assert_eq!(missing, "That PDF couldn’t be opened.");
        std::fs::write(&path, b"not a pdf").unwrap();
        assert_eq!(split(&helper, &path).await.unwrap_err(), "That PDF couldn’t be read.");
    }
}

/// A three-page PDF with one name per page, written by hand, for tests.
#[cfg(test)]
pub(crate) fn tests_pdf() -> Vec<u8> {
    let names = ["Ana Ruiz", "Bo Ruiz", "Chloe Martin"];
    let mut objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        format!("<< /Type /Pages /Kids [{}] /Count 3 >>", (0..3).map(|i| format!("{} 0 R", 4 + i * 2)).collect::<Vec<_>>().join(" ")),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ];
    for name in names {
        let page = objects.len() + 1;
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>", page + 1));
        let text = format!("BT /F1 18 Tf 72 700 Td (Payslip for {name}) Tj ET");
        objects.push(format!("<< /Length {} >>\nstream\n{text}\nendstream", text.len()));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{object}\nendobj\n", index + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
    out
}
