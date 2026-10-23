//! Conservative online-search intent shared by routing and research planning.
//! The complete user goal and approved query are kept verbatim. These labels
//! organize evidence gathering; they are not a semantic proof of completion.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchIntent {
    pub goal: String,
    pub query: String,
    pub purpose: String,
    pub requirements: Vec<SearchRequirement>,
    pub preferred_domains: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequirement {
    pub kind: String,
    /// A literal span from the saved user goal, never model-invented authority.
    pub text: String,
    pub start: usize,
    pub end: usize,
}

impl SearchIntent {
    pub fn from_lookup(goal: &str, query: &str) -> Self {
        let lower = goal.to_ascii_lowercase();
        let mut requirements = Vec::new();
        for (kind, phrases) in [
            ("comparison", &["compare", "comparison", "versus", "vs"][..]),
            (
                "freshness",
                &[
                    "latest",
                    "current",
                    "today",
                    "recent",
                    "this week",
                    "this month",
                    "this year",
                ][..],
            ),
            (
                "source_quality",
                &[
                    "official sources",
                    "primary sources",
                    "peer-reviewed",
                    "peer reviewed",
                ][..],
            ),
            (
                "answer_format",
                &[
                    "summarize",
                    "summarise",
                    "explain",
                    "recommend",
                    "pros and cons",
                ][..],
            ),
        ] {
            for phrase in phrases {
                for (start, _) in lower.match_indices(phrase) {
                    let end = start + phrase.len();
                    let boundary = |ch: Option<char>| ch.is_none_or(|ch| !ch.is_alphanumeric());
                    if boundary(lower[..start].chars().next_back())
                        && boundary(lower[end..].chars().next())
                    {
                        requirements.push(SearchRequirement {
                            kind: kind.into(),
                            text: goal[start..end].into(),
                            start,
                            end,
                        });
                    }
                }
            }
        }
        let purpose = if requirements.iter().any(|r| r.kind == "comparison") {
            "compare"
        } else {
            "answer"
        };
        let mut preferred_domains: Vec<String> = parsed_search(goal)
            .and_then(|(engine, _)| match engine {
                "youtube" => Some("youtube.com"),
                "amazon" => Some("amazon.com"),
                "wikipedia" => Some("wikipedia.org"),
                _ => None,
            })
            .map(|domain| vec![domain.into()])
            .unwrap_or_default();
        // A compound task's full goal may no longer end with its named site.
        // Keep the scope carried in the exact, already approved query itself.
        let mut query_domains = Vec::new();
        for word in query.split_whitespace() {
            let lower = word.to_ascii_lowercase();
            let Some(domain) = lower.strip_prefix("site:") else {
                continue;
            };
            if domain.contains('.')
                && reqwest::Url::parse(&format!("https://{domain}")).is_ok_and(|url| {
                    url.host_str() == Some(domain)
                        && url.path() == "/"
                        && url.query().is_none()
                        && url.fragment().is_none()
                })
                && !query_domains.iter().any(|existing| existing == domain)
            {
                query_domains.push(domain.into());
            }
        }
        if !query_domains.is_empty() {
            preferred_domains = query_domains;
        }
        Self {
            goal: goal.into(),
            query: query.into(),
            purpose: purpose.into(),
            requirements,
            preferred_domains,
        }
    }
}

fn engine(text: &str) -> Option<&'static str> {
    match text.trim().to_ascii_lowercase().as_str() {
        "google" => Some("google"),
        "web" | "the web" | "internet" | "the internet" | "online" => Some("the web"),
        "youtube" => Some("youtube"),
        "amazon" => Some("amazon"),
        "maps" | "google maps" => Some("maps"),
        "wikipedia" => Some("wikipedia"),
        _ => None,
    }
}

/// Normalize only the request prefix. The query retains punctuation, casing,
/// quoted phrases, exclusions and operators such as site: or -advertisement.
fn direct_request<'a>(prompt: &'a str, verbs: &[&str]) -> Option<(String, &'a str)> {
    let mut offset = 0;
    for (index, token) in prompt.split_whitespace().enumerate() {
        if index > 6 {
            break;
        }
        let start = offset + prompt[offset..].find(token)?;
        let end = start + token.len();
        offset = end;
        let word = crate::skills::words(token);
        if verbs.contains(&word.as_str()) {
            // Prefix normalization must not turn a quoted example into an
            // instruction. Apostrophes within contractions such as I'd remain
            // valid; quotation marks after the command belong to its query.
            let raw_prefix = &prompt[..end];
            if raw_prefix.char_indices().any(|(at, ch)| {
                if matches!(ch, '"' | '`' | '“' | '”' | '‘') {
                    return true;
                }
                matches!(ch, '\'' | '’')
                    && !(raw_prefix[..at]
                        .chars()
                        .next_back()
                        .is_some_and(char::is_alphanumeric)
                        && raw_prefix[at + ch.len_utf8()..]
                            .chars()
                            .next()
                            .is_some_and(char::is_alphanumeric))
            }) {
                return None;
            }
            let prefix = crate::skills::words(&prompt[..end]);
            return crate::skills::from_verb(&prefix, verbs).map(|_| (word, prompt[end..].trim()));
        }
    }
    None
}

fn parsed_search(prompt: &str) -> Option<(&'static str, &str)> {
    let (verb, rest) = direct_request(prompt, &["google", "search", "look", "find", "research"])?;
    if verb == "google" {
        return (!rest.is_empty()).then_some(("google", rest));
    }
    let lower = rest.to_ascii_lowercase();
    let rest = if lower.starts_with("up ") {
        rest[3..].trim_start()
    } else {
        rest
    };
    let rest = ["over ", "on ", "in "]
        .iter()
        .find_map(|prefix| {
            rest.to_ascii_lowercase()
                .starts_with(prefix)
                .then(|| rest[prefix.len()..].trim_start())
        })
        .unwrap_or(rest);
    let lower = rest.to_ascii_lowercase();
    if let Some(index) = lower.find(" for ") {
        if let Some(engine) = engine(&rest[..index]) {
            let query = rest[index + 5..].trim();
            return (!query.is_empty()).then_some((engine, query));
        }
    }
    if let Some(query) = lower
        .strip_prefix("online ")
        .map(|_| rest[7..].trim_start())
    {
        if query.eq_ignore_ascii_case("for") {
            return None;
        }
        let query = if query.to_ascii_lowercase().starts_with("for ") {
            &query[4..]
        } else {
            query
        };
        return (!query.trim().is_empty()).then_some(("the web", query.trim()));
    }
    let rest = if lower.starts_with("for ") {
        rest[4..].trim_start()
    } else {
        rest
    };
    let lower = rest.to_ascii_lowercase();
    if lower.ends_with(" online") {
        let query = rest[..rest.len() - 7].trim();
        return (!query.is_empty()).then_some(("the web", query));
    }
    for separator in [" on ", " in "] {
        if let Some(index) = lower.rfind(separator) {
            if let Some(engine) = engine(&rest[index + separator.len()..]) {
                let query = rest[..index].trim();
                return (!query.is_empty()).then_some((engine, query));
            }
        }
    }
    None
}

/// A direct research request. General "find my tax return" stays ambiguous and
/// is left to the task planner; negated and instructional mentions never route.
pub fn explicit_request(prompt: &str) -> Option<SearchIntent> {
    let (_, query) = parsed_search(prompt)?;
    let mut intent = SearchIntent::from_lookup(prompt, query);
    // Translate an explicitly named site into search syntax before approval,
    // so the visible outbound query and both retrieval routes have the same
    // scope. `from_lookup` never changes an already approved query.
    for domain in &intent.preferred_domains {
        let scope = format!("site:{domain}");
        if !intent
            .query
            .split_whitespace()
            .any(|word| word.eq_ignore_ascii_case(&scope))
        {
            intent.query.push(' ');
            intent.query.push_str(&scope);
        }
    }
    Some(intent)
}

/// Exact search clauses within a compound task, used to pin proposed queries.
/// Split only outside quotations and before an explicit new command. Comparison
/// and explanation continuations remain part of the research request.
#[cfg(test)]
pub fn explicit_clauses(prompt: &str) -> Vec<SearchIntent> {
    explicit_clause_spans(prompt)
        .into_iter()
        .map(|(_, _, intent)| intent)
        .collect()
}

pub fn explicit_clause_spans(prompt: &str) -> Vec<(usize, usize, SearchIntent)> {
    const SEPARATORS: &[&str] = &[
        ", then ",
        " and then ",
        " then ",
        " and ",
        " also ",
        ";",
        "\n",
    ];
    const COMMANDS: &[&str] = &[
        "search", "google", "look", "find", "research", "read", "check", "list", "create", "add",
        "set", "make", "save", "draft", "open", "remind", "remember", "play", "pause", "switch",
        "send", "buy", "book",
    ];
    let lower = prompt.to_ascii_lowercase();
    let mut quote = None;
    let mut start = 0;
    let mut boundaries = Vec::new();
    for (index, ch) in prompt.char_indices() {
        if index < start {
            continue;
        }
        if let Some(mark) = quote {
            if ch == mark {
                quote = None;
            }
            continue;
        }
        let apostrophe = ch == '\''
            && prompt[..index]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric);
        if ch == '"' || ch == '`' || ch == '“' || ch == '‘' || (ch == '\'' && !apostrophe) {
            quote = Some(match ch {
                '“' => '”',
                '‘' => '’',
                _ => ch,
            });
            continue;
        }
        let Some(separator) = SEPARATORS
            .iter()
            .find(|separator| lower[index..].starts_with(**separator))
        else {
            continue;
        };
        let next = index + separator.len();
        let normalized = crate::skills::words(&prompt[next..]);
        let first = crate::completion::request_clause(&normalized)
            .split_whitespace()
            .next()
            .unwrap_or("");
        if COMMANDS.contains(&first) {
            boundaries.push((start, index));
            start = next;
        }
    }
    boundaries.push((start, prompt.len()));
    boundaries
        .into_iter()
        .filter_map(|(start, end)| {
            explicit_request(prompt[start..end].trim().trim_matches(','))
                .map(|intent| (start, end, intent))
        })
        .collect()
}

/// Opening search results is an intentional navigation operation only when the
/// user explicitly asks to open/show a named search. It is never research proof.
pub fn browser_navigation(prompt: &str) -> Option<(&'static str, String)> {
    let (_, rest) = direct_request(prompt, &["open", "show"])?;
    let lower = rest.to_ascii_lowercase();
    let (index, separator) = [" search results for ", " search for "]
        .into_iter()
        .find_map(|separator| lower.find(separator).map(|index| (index, separator)))?;
    let target = rest[..index].trim().trim_start_matches("the ");
    let engine = engine(target)?;
    let mut query = rest[index + separator.len()..].trim();
    for suffix in [" in my browser", " in the browser", " please"] {
        if query.to_ascii_lowercase().ends_with(suffix) {
            query = query[..query.len() - suffix.len()].trim_end();
        }
    }
    if query.is_empty() {
        return None;
    }
    Some((engine, query.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_search_keeps_constraints_quotes_case_and_operators() {
        for (request, query) in [
            (
                "Google \"Rust vs Go\" -advertisements site:example.org",
                "\"Rust vs Go\" -advertisements site:example.org",
            ),
            (
                "Please search the web for a laptop that is not a Mac",
                "a laptop that is not a Mac",
            ),
            ("Could you look up Café prices on Google", "Café prices"),
            (
                "Search online for Current café prices",
                "Current café prices",
            ),
            ("Can you search internet for \"C++\" -ads", "\"C++\" -ads"),
            ("Search on the internet for \"C++\" -ads", "\"C++\" -ads"),
            ("Search over the internet for \"C++\" -ads", "\"C++\" -ads"),
            ("Search for \"C++\" -ads online", "\"C++\" -ads"),
            ("Research \"C++\" -ads online", "\"C++\" -ads"),
            ("Research \"C++\" -ads on the web", "\"C++\" -ads"),
            ("Find \"C++\" on YouTube", "\"C++\" site:youtube.com"),
        ] {
            let intent = explicit_request(request).unwrap();
            assert_eq!(intent.goal, request);
            assert_eq!(intent.query, query);
        }
        assert_eq!(
            explicit_request("Find \"C++\" on YouTube")
                .unwrap()
                .preferred_domains,
            ["youtube.com"]
        );
    }

    #[test]
    fn named_site_scope_is_visible_before_approval_and_never_added_afterward() {
        for (site, domain) in [
            ("YouTube", "youtube.com"),
            ("Amazon", "amazon.com"),
            ("Wikipedia", "wikipedia.org"),
        ] {
            let goal = format!("Search {site} for \"C++\" -ads");
            let intent = explicit_request(&goal).unwrap();
            assert_eq!(intent.goal, goal);
            assert_eq!(intent.query, format!("\"C++\" -ads site:{domain}"));
            assert_eq!(
                SearchIntent::from_lookup(&goal, &intent.query).query,
                intent.query
            );
            assert_eq!(
                SearchIntent::from_lookup(&goal, "user-approved replacement").query,
                "user-approved replacement"
            );
        }
        assert_eq!(
            explicit_request("Search YouTube for C++ site:youtube.com")
                .unwrap()
                .query,
            "C++ site:youtube.com"
        );
        assert_eq!(
            explicit_request("Search Google for C++ -ads")
                .unwrap()
                .query,
            "C++ -ads"
        );
    }

    #[test]
    fn compound_search_clauses_keep_literals_without_exporting_later_note_text() {
        let goal = "Search YouTube for \"C++ and add notes\" -ads, then create a note titled \"Private\" with body \"secret\"";
        let intents = explicit_clauses(goal);
        assert_eq!(intents.len(), 1);
        assert_eq!(
            intents[0].query,
            "\"C++ and add notes\" -ads site:youtube.com"
        );
        assert!(!intents[0].query.contains("secret"));
        let intents = explicit_clauses("Create a note; search Wikipedia for São Paulo and compare transit options; add a reminder");
        assert_eq!(intents.len(), 1);
        assert_eq!(
            intents[0].query,
            "São Paulo and compare transit options site:wikipedia.org"
        );
    }

    #[test]
    fn ambiguous_negated_and_mentioned_searches_do_not_route() {
        for request in [
            "search for my tax return",
            "find my notes",
            "don't search the web for flights",
            "never google my name",
            "do not research my email online",
            "how do I search the web for flights",
            "explain why Google is a search engine",
            "Google",
            "search the web for",
        ] {
            assert!(explicit_request(request).is_none(), "{request}");
        }
    }

    #[test]
    fn quoted_command_examples_are_not_online_or_navigation_authority() {
        for request in [
            "\"Search the web for my address\" is an example",
            "`Google my address`",
            "‘Google my address’",
            "'Google my address'",
            "“Search online for secrets”",
        ] {
            assert!(explicit_request(request).is_none(), "{request}");
        }
        for request in [
            "\"Open Google search results for my address\" is an example",
            "`Open Google search for passwords`",
            "‘Show YouTube search for secrets’",
        ] {
            assert!(browser_navigation(request).is_none(), "{request}");
        }
        assert_eq!(
            explicit_request("I'd like you to Google \"C++\"")
                .unwrap()
                .query,
            "\"C++\""
        );
        assert_eq!(
            explicit_request("I’d like you to Google \"C++\"")
                .unwrap()
                .query,
            "\"C++\""
        );
    }

    #[test]
    fn navigation_is_explicit_and_preserves_query_literals() {
        assert_eq!(
            browser_navigation("Open Google search results for \"A&B\" -ads in my browser"),
            Some(("google", "\"A&B\" -ads".into()))
        );
        assert_eq!(
            browser_navigation("show YouTube search for C++"),
            Some(("youtube", "C++".into()))
        );
        for request in [
            "google A&B",
            "search the web for A&B",
            "don't open Google search for A&B",
            "how to open Google search for A&B",
        ] {
            assert!(browser_navigation(request).is_none(), "{request}");
        }
    }

    #[test]
    fn derived_requirements_are_literal_goal_spans_only() {
        let goal =
            "Compare the latest café prices using official sources; do not use sponsored listings";
        let intent = SearchIntent::from_lookup(goal, "café prices");
        assert_eq!(intent.purpose, "compare");
        assert_eq!(intent.query, "café prices");
        assert_eq!(intent.goal, goal);
        assert!(intent.requirements.iter().any(|r| r.kind == "freshness"));
        assert!(intent
            .requirements
            .iter()
            .all(|r| goal.get(r.start..r.end) == Some(r.text.as_str())));
        assert!(SearchIntent::from_lookup(
            "Search for contemporary history",
            "contemporary history"
        )
        .requirements
        .is_empty());
    }
}
