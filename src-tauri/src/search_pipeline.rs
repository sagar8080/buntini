//! One foreground research policy, independent of model tool choice. Search
//! results are discovery; only selected, relevant page readbacks are evidence.
use crate::{research::ResearchContent, search_intent::SearchIntent, types::Source};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub type SearchFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;
pub trait Backend {
    fn browser_search<'a>(&'a mut self, query: &'a str) -> SearchFuture<'a, Vec<Source>>;
    fn api_search<'a>(&'a mut self, query: &'a str) -> SearchFuture<'a, Vec<Source>>;
    fn read_page<'a>(
        &'a mut self,
        selected: &'a [String],
        url: &'a str,
    ) -> SearchFuture<'a, ResearchContent>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attempt {
    pub route: String,
    pub status: String,
    pub detail: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub intent: SearchIntent,
    pub attempts: Vec<Attempt>,
    pub sources: Vec<Source>,
    pub pages: Vec<ResearchContent>,
    pub limitations: Vec<String>,
    /// Retrieval sufficiency only; never a claim that every user outcome is done.
    pub evidence_ready: bool,
    pub answer_verified: bool,
}

fn tokens(text: &str) -> HashSet<String> {
    const STOP: &[&str] = &[
        "a",
        "an",
        "and",
        "are",
        "as",
        "at",
        "be",
        "by",
        "can",
        "compare",
        "comparison",
        "for",
        "from",
        "how",
        "i",
        "in",
        "is",
        "it",
        "me",
        "of",
        "on",
        "or",
        "please",
        "search",
        "the",
        "to",
        "using",
        "vs",
        "versus",
        "what",
        "which",
        "with",
        "www",
        "com",
        "https",
        "latest",
        "current",
        "today",
        "online",
        "web",
    ];
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() > 1)
        .map(str::to_lowercase)
        .filter(|s| !STOP.contains(&s.as_str()))
        .collect()
}
fn overlap(query: &str, text: &str) -> usize {
    tokens(query).intersection(&tokens(text)).count()
}
fn host(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h.trim_start_matches("www.").to_string())
        })
        .unwrap_or_default()
}
fn relevant(intent: &SearchIntent, text: &str) -> bool {
    tokens(&intent.query).is_empty() || overlap(&intent.query, text) > 0
}
fn preferred(intent: &SearchIntent, url: &str) -> bool {
    let h = host(url);
    intent.preferred_domains.is_empty()
        || intent
            .preferred_domains
            .iter()
            .any(|d| h == *d || h.ends_with(&format!(".{d}")))
}
pub(crate) fn usable_page(intent: &SearchIntent, page: &ResearchContent) -> bool {
    let lower = page.text.to_ascii_lowercase();
    page.text.chars().count() >= 120
        && relevant(intent, &format!("{} {}", page.title, page.text))
        && ![
            "verify you are human",
            "verifying you're not a bot",
            "verifying you’re not a bot",
            "switch to traditional captcha",
            "quick check before you continue searching",
            "checking your browser",
            "enable javascript and cookies to continue",
            "just a moment...",
            "access denied",
            "unusual traffic",
        ]
        .iter()
        .any(|s| lower.contains(s))
}
fn source_identity(intent: &SearchIntent, url: &str) -> String {
    // A user-scoped comparison such as Amazon product A versus B needs two
    // distinct pages on that site. Requiring another host would contradict the
    // approved scope. Broader comparisons retain the distinct-host threshold.
    if intent.preferred_domains.len() == 1 {
        url.into()
    } else {
        host(url)
    }
}
fn enough(report: &Report) -> bool {
    let sources: HashSet<_> = report
        .pages
        .iter()
        .map(|p| source_identity(&report.intent, &p.url))
        .collect();
    if report.intent.purpose == "compare" {
        sources.len() >= 2
    } else {
        !sources.is_empty()
    }
}
fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Task cancelled.".into())
    } else {
        Ok(())
    }
}

/// Exact approved query goes to both backends. Site content cannot change it,
/// select another API provider, add a subquery or authorize an external action.
pub async fn run<B: Backend>(
    intent: SearchIntent,
    backend: &mut B,
    cancel: Arc<AtomicBool>,
) -> Result<Report, String> {
    let report = Report {
        intent,
        attempts: vec![],
        sources: vec![],
        pages: vec![],
        limitations: vec![],
        evidence_ready: false,
        answer_verified: false,
    };
    run_routes(report, backend, cancel, &["browser", "api_fallback"]).await
}

/// If synthesis identifies a missing requirement after usable browser reads,
/// use the same approved fallback once, retaining already-read evidence.
pub async fn complete_with_api<B: Backend>(
    mut report: Report,
    backend: &mut B,
    cancel: Arc<AtomicBool>,
) -> Result<Report, String> {
    if report.attempts.iter().any(|a| a.route == "api_fallback") {
        return Ok(report);
    }
    report.answer_verified = false;
    report.limitations.clear();
    for attempt in &mut report.attempts {
        if attempt.route == "synthesis" {
            attempt.status = "previous_insufficient".into();
        }
    }
    run_routes(report, backend, cancel, &["api_fallback"]).await
}

async fn run_routes<B: Backend>(
    mut report: Report,
    backend: &mut B,
    cancel: Arc<AtomicBool>,
    routes: &[&str],
) -> Result<Report, String> {
    let mut seen: HashSet<String> = report
        .attempts
        .iter()
        .filter(|a| a.route == "read_source")
        .map(|a| a.detail.split(": ").next().unwrap_or_default().to_string())
        .collect();
    let mut read_attempts = report
        .attempts
        .iter()
        .filter(|a| a.route == "read_source")
        .count();
    for &route in routes {
        check_cancel(&cancel)?;
        let found = if route == "browser" {
            backend.browser_search(&report.intent.query).await
        } else {
            backend.api_search(&report.intent.query).await
        };
        check_cancel(&cancel)?;
        let mut found = match found {
            Ok(sources) => sources,
            Err(error) if error == "Task cancelled." => return Err(error),
            Err(error) => {
                report.attempts.push(Attempt {
                    route: route.into(),
                    status: "unavailable".into(),
                    detail: error,
                });
                continue;
            }
        };
        found.retain(|s| {
            crate::research::validate_public_url(&s.url).is_ok()
                && preferred(&report.intent, &s.url)
        });
        found.sort_by_cached_key(|s| {
            std::cmp::Reverse(overlap(
                &report.intent.query,
                &format!("{} {} {}", s.title, s.snippet, s.url),
            ))
        });
        // Deduplicate fragments and diversify domains before reading comparison evidence.
        let mut batch = Vec::new();
        for mut source in found.into_iter().take(20) {
            let mut url = reqwest::Url::parse(&source.url).unwrap();
            url.set_fragment(None);
            source.url = url.to_string();
            if !seen.contains(&source.url) && !batch.iter().any(|s: &Source| s.url == source.url) {
                batch.push(source);
            }
        }
        if report.intent.purpose == "compare" {
            let mut hosts = HashSet::new();
            let mut duplicates = Vec::new();
            let mut diverse = Vec::new();
            for s in batch {
                if hosts.insert(host(&s.url)) {
                    diverse.push(s);
                } else {
                    duplicates.push(s);
                }
            }
            diverse.extend(duplicates);
            batch = diverse;
        }
        let before = report.pages.len();
        let selected: Vec<String> = batch.iter().map(|s| s.url.clone()).collect();
        // Three candidates per route, six total. Never follow arbitrary page links.
        for source in batch.into_iter().take(3) {
            if read_attempts >= 6 {
                break;
            }
            check_cancel(&cancel)?;
            read_attempts += 1;
            seen.insert(source.url.clone());
            let page = backend.read_page(&selected, &source.url).await;
            check_cancel(&cancel)?;
            let url = source.url.clone();
            if let Some(existing) = report.sources.iter_mut().find(|s| s.url == source.url) {
                *existing = source;
            } else {
                report.sources.push(source);
            }
            match page {
                Ok(page) if page.url == url && usable_page(&report.intent, &page) => {
                    report.attempts.push(Attempt {route:"read_source".into(),status:"read".into(),detail:url});
                    report.pages.push(page);
                }
                Ok(_) => report.attempts.push(Attempt {route:"read_source".into(),status:"insufficient".into(),detail:format!("{url}: the readback was too short, unrelated, blocked, or did not match the selected URL.")}),
                Err(error) if error == "Task cancelled." => return Err(error),
                Err(error) => report.attempts.push(Attempt {route:"read_source".into(),status:"unavailable".into(),detail:format!("{url}: {error}")}),
            }
        }
        report.attempts.push(Attempt {
            route: route.into(),
            status: if enough(&report) {
                "evidence_ready"
            } else {
                "insufficient"
            }
            .into(),
            detail: format!("Read {} usable pages.", report.pages.len() - before),
        });
        if enough(&report) {
            break;
        }
    }
    report.evidence_ready = enough(&report);
    if !report.evidence_ready {
        report.limitations.push(if report.intent.purpose == "compare" { if report.intent.preferred_domains.len() == 1 { "The comparison needs relevant evidence from at least two distinct pages on the requested site." } else { "The comparison needs relevant page evidence from at least two distinct sites." } } else { "No selected result supplied enough relevant, readable page content." }.into());
    }
    if report.pages.iter().any(|p| p.truncated) {
        report.limitations.push(
            "Some source pages were truncated; their unread sections cannot support the answer."
                .into(),
        );
    }
    Ok(report)
}

/// Structural check, not an entailment claim: all numbered references must
/// resolve to pages actually read. Comparisons cite distinct sites, or distinct
/// page URLs when the user's approved query explicitly selects one site.
pub fn validate_answer(answer: &str, report: &Report) -> Result<(), String> {
    if !report.evidence_ready || answer.trim().is_empty() {
        return Err("The source evidence is incomplete.".into());
    }
    let mut cited = HashSet::new();
    let mut rest = answer;
    while let Some(start) = rest.find('[') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find(']') else {
            break;
        };
        let marker = &rest[..end];
        if marker.starts_with(|c: char| c.is_ascii_digit())
            && !marker.chars().all(|c| c.is_ascii_digit())
        {
            return Err("Use separate numbered citations such as [1] [2].".into());
        }
        if !marker.is_empty() && marker.chars().all(|c| c.is_ascii_digit()) {
            let index = marker
                .parse::<usize>()
                .ok()
                .and_then(|i| i.checked_sub(1))
                .ok_or("The summary contains an invalid citation.")?;
            let source = report
                .sources
                .get(index)
                .ok_or("The summary cites a source that was not retrieved.")?;
            if !report.pages.iter().any(|p| p.url == source.url) {
                return Err(
                    "The summary cites a search snippet instead of a page that was read.".into(),
                );
            }
            cited.insert(source_identity(&report.intent, &source.url));
        }
        rest = &rest[end + 1..];
    }
    if cited.len()
        < if report.intent.purpose == "compare" {
            2
        } else {
            1
        }
    {
        return Err("The answer does not cite enough of the pages read.".into());
    }
    Ok(())
}

/// The synthesis model must explicitly account for missing requirements. Its
/// text is buffered until this structural/citation gate passes.
pub fn accept_summary(raw: &str, report: &mut Report) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Claim {
        text: String,
        sources: Vec<String>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Summary {
        #[serde(default)]
        answer: Option<String>,
        #[serde(default)]
        claims: Option<Vec<Claim>>,
        coverage: String,
        missing: Vec<String>,
    }
    let raw = raw
        .trim()
        .strip_prefix("```json")
        .and_then(|s| s.strip_suffix("```"))
        .unwrap_or(raw)
        .trim();
    let summary: Summary = serde_json::from_str(raw).map_err(|_| {
        "The research summary did not provide a verifiable coverage result.".to_string()
    })?;
    if summary.coverage != "complete" || !summary.missing.is_empty() {
        report.attempts.push(Attempt {
            route: "synthesis".into(),
            status: "insufficient".into(),
            detail: "The sources did not answer all research requirements.".into(),
        });
        report.limitations.extend(
            summary
                .missing
                .into_iter()
                .take(12)
                .map(|s| s.chars().take(500).collect::<String>()),
        );
        return Err(
            "The retrieved sources do not yet answer all the research requirements.".into(),
        );
    }
    let answer = match (summary.answer, summary.claims) {
        (None, Some(claims)) if !claims.is_empty() && claims.len() <= 30 => {
            let mut lines = Vec::new();
            for claim in claims {
                if claim.text.trim().is_empty()
                    || claim.sources.is_empty()
                    || claim.sources.len() > 6
                {
                    return Err(
                        "Every research claim needs text and at least one read source.".into(),
                    );
                }
                let citations = claim
                    .sources
                    .iter()
                    .map(|n| {
                        if n.is_empty()
                            || !n.chars().all(|c| c.is_ascii_digit())
                            || !n
                                .parse::<usize>()
                                .is_ok_and(|i| i > 0 && i.to_string() == *n)
                        {
                            Err("A claim contains an invalid source number.".to_string())
                        } else {
                            Ok(format!("[{n}]"))
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .join(" ");
                lines.push(format!("{} {}", claim.text.trim(), citations));
            }
            lines.join("\n\n")
        }
        (Some(answer), None) => answer,
        _ => return Err("The summary must supply either sourced claims or a cited answer.".into()),
    };
    validate_answer(&answer, report)?;
    let answer =
        crate::completion::validate_answer("Answer using retrieved evidence.", &answer, &[])?;
    report.answer_verified = true;
    Ok(answer)
}

impl Report {
    /// Compact receipt for the durable workflow, without duplicating page bodies.
    pub fn proof(&self) -> serde_json::Value {
        serde_json::json!({"intent":self.intent,"attempts":self.attempts,"limitations":self.limitations,"evidenceReady":self.evidence_ready,"answerVerified":self.answer_verified})
    }
}

#[cfg(test)]
mod site_comparison_tests {
    use super::*;

    fn report(query: &str, urls: &[&str]) -> Report {
        let intent = SearchIntent::from_lookup("Compare two product options", query);
        Report {
            intent,
            sources: urls
                .iter()
                .map(|url| Source {
                    title: "Product option".into(),
                    url: (*url).into(),
                    snippet: String::new(),
                    retrieved_at: "2026-10-22T12:00:00Z".into(),
                })
                .collect(),
            pages: urls
                .iter()
                .map(|url| ResearchContent {
                    url: (*url).into(),
                    title: "Product option".into(),
                    text: "A retrieved product specification.".into(),
                    retrieved_at: "2026-10-22T12:00:00Z".into(),
                    truncated: false,
                })
                .collect(),
            attempts: vec![],
            limitations: vec![],
            evidence_ready: true,
            answer_verified: false,
        }
    }

    #[test]
    fn explicit_single_site_comparison_requires_two_distinct_read_and_cited_pages() {
        let report = report(
            "Compare product A vs B site:amazon.com",
            &["https://amazon.com/dp/A", "https://amazon.com/dp/B"],
        );
        assert!(enough(&report));
        assert!(validate_answer("Product A [1] differs from product B [2].", &report).is_ok());
        assert!(validate_answer("Product A [1] differs from product B [1].", &report).is_err());
        let mut duplicate = report.clone();
        duplicate.pages[1] = duplicate.pages[0].clone();
        duplicate.sources[1] = duplicate.sources[0].clone();
        assert!(!enough(&duplicate));
        assert!(validate_answer("Product A [1] differs from product B [2].", &duplicate).is_err());
    }

    #[test]
    fn unrestricted_comparison_still_requires_distinct_sites() {
        let same_site = report(
            "Compare product A vs B",
            &["https://amazon.com/dp/A", "https://amazon.com/dp/B"],
        );
        assert!(!enough(&same_site));
        assert!(validate_answer("Product A [1] differs from product B [2].", &same_site).is_err());
        let different_sites = report(
            "Compare product A vs B",
            &[
                "https://amazon.com/dp/A",
                "https://manufacturer.org/product-b",
            ],
        );
        assert!(enough(&different_sites));
        assert!(validate_answer(
            "Product A [1] differs from product B [2].",
            &different_sites
        )
        .is_ok());
    }
}
