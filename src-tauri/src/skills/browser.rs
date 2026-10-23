//! Browser and apps: open a website, app, or explicitly requested search-results
//! page, list or switch tabs, close the current tab, and read the page
//! you're on so Buntini can summarise it or answer from it. Pages are read from the browser
//! itself (Safari's own scripting, or the accessibility API for Chrome, Brave, Edge and
//! Arc), never fetched again online.
use super::{from_verb, osascript, without_politeness, Context, Outcome};
use crate::privacy::Share;

#[derive(Debug, PartialEq)]
pub enum Command {
    OpenSite {
        name: String,
        url: String,
    },
    OpenApp(String),
    Search {
        engine: &'static str,
        query: String,
    },
    /// Summarise or answer from the page showing in the browser.
    Page,
    CloseTab,
    Tabs,
    SwitchTab(String),
}

/// Sites people open by name.
const SITES: [(&str, &str, &str); 34] = [
    ("youtube", "YouTube", "https://www.youtube.com"),
    ("gmail", "Gmail", "https://mail.google.com"),
    ("google", "Google", "https://www.google.com"),
    ("google maps", "Google Maps", "https://www.google.com/maps"),
    ("maps", "Google Maps", "https://www.google.com/maps"),
    ("google drive", "Google Drive", "https://drive.google.com"),
    ("drive", "Google Drive", "https://drive.google.com"),
    ("google docs", "Google Docs", "https://docs.google.com"),
    (
        "google calendar",
        "Google Calendar",
        "https://calendar.google.com",
    ),
    (
        "google photos",
        "Google Photos",
        "https://photos.google.com",
    ),
    ("github", "GitHub", "https://github.com"),
    ("netflix", "Netflix", "https://www.netflix.com"),
    ("reddit", "Reddit", "https://www.reddit.com"),
    ("twitter", "X", "https://x.com"),
    ("x", "X", "https://x.com"),
    ("linkedin", "LinkedIn", "https://www.linkedin.com"),
    ("facebook", "Facebook", "https://www.facebook.com"),
    ("instagram", "Instagram", "https://www.instagram.com"),
    ("amazon", "Amazon", "https://www.amazon.com"),
    ("wikipedia", "Wikipedia", "https://www.wikipedia.org"),
    ("chatgpt", "ChatGPT", "https://chatgpt.com"),
    ("claude", "Claude", "https://claude.ai"),
    ("whatsapp", "WhatsApp Web", "https://web.whatsapp.com"),
    ("spotify", "Spotify", "https://open.spotify.com"),
    ("notion", "Notion", "https://www.notion.so"),
    ("figma", "Figma", "https://www.figma.com"),
    ("discord", "Discord", "https://discord.com/app"),
    ("twitch", "Twitch", "https://www.twitch.tv"),
    ("hacker news", "Hacker News", "https://news.ycombinator.com"),
    (
        "stack overflow",
        "Stack Overflow",
        "https://stackoverflow.com",
    ),
    ("bbc", "BBC", "https://www.bbc.com"),
    ("the news", "Google News", "https://news.google.com"),
    ("google news", "Google News", "https://news.google.com"),
    (
        "weather",
        "the weather",
        "https://www.google.com/search?q=weather",
    ),
];

const ENGINES: [(&str, &str, &str); 6] = [
    ("google", "Google", "https://www.google.com/search?q="),
    ("the web", "Google", "https://www.google.com/search?q="),
    (
        "youtube",
        "YouTube",
        "https://www.youtube.com/results?search_query=",
    ),
    ("amazon", "Amazon", "https://www.amazon.com/s?k="),
    ("maps", "Google Maps", "https://www.google.com/maps/search/"),
    (
        "wikipedia",
        "Wikipedia",
        "https://en.wikipedia.org/w/index.php?search=",
    ),
];

/// An installed app by name, ignoring case ("open figma", "launch the notes app").
fn installed_app(name: &str) -> Option<String> {
    let name = name
        .trim()
        .trim_start_matches("the ")
        .trim_end_matches(" app")
        .trim();
    if name.is_empty() {
        return None;
    }
    crate::apps::app_names()
        .into_iter()
        .find(|app| app.to_lowercase() == name)
}

pub fn recognise(text: &str) -> Option<Command> {
    // Reading or closing a page must be an actual request, rather than an incidental
    // mention (or "don't close this tab").
    let page_targets = [
        "this page",
        "this article",
        "this tab",
        "this site",
        "this website",
        "the page",
        "the article",
        "this post",
    ];
    if from_verb(text, &["summarise", "summarize", "read", "explain"]).is_some_and(
        |(verb, rest)| {
            let target = without_politeness(rest);
            page_targets.contains(&target)
                || (target == "this" && ["summarise", "summarize"].contains(&verb))
        },
    ) || [
        "tldr",
        "tl dr",
        "what's this article about",
        "what is this article about",
        "what's this page about",
        "what is this page about",
    ]
    .contains(&text)
    {
        return Some(Command::Page);
    }
    if from_verb(text, &["close"]).is_some_and(|(_, rest)| {
        [
            "this tab",
            "the tab",
            "tab",
            "the current tab",
            "current tab",
        ]
        .contains(&without_politeness(rest))
    }) {
        return Some(Command::CloseTab);
    }
    if [
        "what tabs do i have open",
        "which tabs do i have open",
        "what tabs are open",
        "which tabs are open",
        "my open tabs",
    ]
    .contains(&text)
        || from_verb(text, &["list", "show"]).is_some_and(|(_, rest)| {
            [
                "my tabs",
                "my open tabs",
                "open tabs",
                "the open tabs",
                "tabs",
            ]
            .contains(&without_politeness(rest))
        })
    {
        return Some(Command::Tabs);
    }
    if let Some((_, rest)) = from_verb(text, &["switch", "go", "jump", "open"]) {
        let rest = rest
            .trim_start_matches("back ")
            .trim_start_matches("to ")
            .trim_start_matches("my ")
            .trim_start_matches("the ");
        if let Some(name) = rest
            .strip_suffix(" tab")
            .filter(|name| !name.is_empty() && name.split(' ').count() <= 5)
        {
            return Some(Command::SwitchTab(name.to_string()));
        }
    }
    // Research requests always enter the integrated online workflow. Only the
    // raw-prompt parser below recognizes explicit search-page navigation.
    // Opening a site or an app: "open youtube", "go to github.com", "launch figma".
    if let Some((_, rest)) = from_verb(text, &["open", "launch", "go", "take", "visit", "start"]) {
        let target = rest
            .trim_start_matches("me ")
            .trim_start_matches("to ")
            .trim_start_matches("up ")
            .trim_end_matches(" for me")
            .trim_end_matches(" please")
            .trim();
        if target.is_empty() || target.split(' ').count() > 4 {
            return None;
        }
        if let Some(app) = installed_app(target) {
            return Some(Command::OpenApp(app));
        }
        let site = target
            .trim_start_matches("the ")
            .trim_end_matches(" website")
            .trim_end_matches(" site")
            .trim_end_matches(".com")
            .trim();
        if let Some((_, name, url)) = SITES
            .iter()
            .find(|(key, _, _)| *key == site || *key == target)
        {
            return Some(Command::OpenSite {
                name: name.to_string(),
                url: url.to_string(),
            });
        }
        // A web address: "github.com", "bbc.co.uk/news".
        if let Some(command) = open_address(target) {
            return Some(command);
        }
    }
    None
}

fn open_address(address: &str) -> Option<Command> {
    if address.chars().any(char::is_whitespace) {
        return None;
    }
    let url = if address.contains("://") {
        address.to_string()
    } else {
        format!("https://{address}")
    };
    let parsed = reqwest::Url::parse(&url).ok()?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || !parsed
            .host_str()
            .is_some_and(|host| host.contains('.') && !host.ends_with('.'))
    {
        return None;
    }
    let url = if parsed.path() == "/" && parsed.query().is_none() && parsed.fragment().is_none() {
        parsed.to_string().trim_end_matches('/').to_string()
    } else {
        parsed.to_string()
    };
    Some(Command::OpenSite {
        name: address.to_string(),
        url,
    })
}

/// Preserve URL paths, query parameters and their case before general word
/// normalisation removes punctuation. Only the command prefix is normalised.
pub fn recognise_prompt(prompt: &str) -> Option<Command> {
    if let Some((engine, query)) = crate::search_intent::browser_navigation(prompt) {
        return Some(Command::Search { engine, query });
    }
    let tokens: Vec<&str> = prompt.split_whitespace().collect();
    for (index, token) in tokens.iter().enumerate().take(7) {
        let verb = super::words(token);
        if !["open", "visit", "go", "take"].contains(&verb.as_str()) {
            continue;
        }
        let prefix = super::words(&tokens[..=index].join(" "));
        if from_verb(&prefix, &[verb.as_str()]).is_none() {
            break;
        }
        let mut target = tokens[index + 1..].join(" ");
        for lead in ["me ", "to "] {
            if target.to_lowercase().starts_with(lead) {
                target = target[lead.len()..].to_string();
            }
        }
        let target = without_politeness(&target).trim();
        if target.contains(['/', '?', '#', ':', '&', '=', '%']) {
            return open_address(target);
        }
        break;
    }
    recognise(&super::words(prompt))
}

/// Bundle identifiers, for reading pages through the accessibility API.
fn bundle(app_id: &str) -> Option<&'static str> {
    match app_id {
        "chrome" => Some("com.google.Chrome"),
        "brave" => Some("com.brave.Browser"),
        "edge" => Some("com.microsoft.edgemac"),
        "arc" => Some("company.thebrowser.Browser"),
        "safari" => Some("com.apple.Safari"),
        _ => None,
    }
}

/// The browser a request about "this page" or "this tab" means: the one in front before
/// Buntini opened if it's a browser, otherwise the first allowed browser that's open.
fn current_browser(context: &Context) -> Result<&'static crate::apps::App, String> {
    let browsers: Vec<&crate::apps::App> = crate::apps::APPS
        .iter()
        .filter(|app| crate::apps::BROWSERS.contains(&app.id) && crate::apps::installed(app))
        .collect();
    let front = context
        .front_app
        .as_deref()
        .and_then(|name| browsers.iter().find(|app| app.name == name))
        .copied();
    let chosen = front
        .or_else(|| {
            browsers
                .iter()
                .find(|app| {
                    crate::apps::allowed(context.settings, app.id)
                        && bundle(app.id).and_then(crate::ax::pid).is_some()
                })
                .copied()
        })
        .ok_or("Open the page in Safari, Chrome, Brave, Edge or Arc first.")?;
    if !crate::apps::allowed(context.settings, chosen.id) {
        return Err(format!(
            "Allow {} in Settings → Apps so I can read its tabs.",
            chosen.name
        ));
    }
    Ok(chosen)
}

async fn open(target: &str) -> Result<(), String> {
    let mut command = tokio::process::Command::new("/usr/bin/open");
    command.arg(target);
    crate::actions::output(command, 15).await.map(|_| ())
}

pub async fn run(command: Command, context: &Context<'_>) -> Result<Outcome, String> {
    let done = |answer: String, share: Option<Share>| Ok(Outcome::Done { answer, share });
    match command {
        Command::OpenSite { name, url } => {
            open(&url).await?;
            done(format!("Opened {name} in your browser."), None)
        }
        Command::OpenApp(name) => {
            let path = crate::actions::find_app(&name)
                .ok_or_else(|| format!("I couldn’t find {name} in Applications."))?;
            open(&path.to_string_lossy()).await?;
            done(format!("Opened {name}."), None)
        }
        Command::Search { engine, query } => {
            let (_, name, base) = ENGINES
                .iter()
                .find(|(key, _, _)| *key == engine)
                .ok_or("Unknown search.")?;
            open(&format!("{base}{}", crate::apps::percent(&query))).await?;
            done(
                format!("Opened {name} search results for “{query}” in your browser."),
                None,
            )
        }
        Command::Tabs => {
            let browsers: Vec<&crate::apps::App> = crate::apps::usable(context.settings)
                .into_iter()
                .filter(|app| crate::apps::BROWSERS.contains(&app.id))
                .collect();
            if browsers.is_empty() {
                return Err("Allow your browser in Settings → Apps so I can see its tabs.".into());
            }
            done(
                crate::apps::browser_tabs(&browsers, "").await?,
                Some(Share::Browser),
            )
        }
        Command::SwitchTab(name) => {
            let browsers: Vec<&crate::apps::App> = crate::apps::usable(context.settings)
                .into_iter()
                .filter(|app| crate::apps::BROWSERS.contains(&app.id))
                .collect();
            if browsers.is_empty() {
                return Err(
                    "Allow your browser in Settings → Apps so I can switch its tabs.".into(),
                );
            }
            done(
                crate::apps::switch_tab(&browsers, &name).await?,
                Some(Share::Browser),
            )
        }
        Command::CloseTab => {
            let browser = current_browser(context)?;
            crate::apps::ensure_public_window(browser).await?;
            let script = match browser.id {
                "safari" => {
                    r#"tell application "Safari" to close current tab of front window"#.to_string()
                }
                "arc" => {
                    return Err(
                        "Arc doesn’t let other apps close its tabs. Press ⌘W in Arc.".into(),
                    )
                }
                _ => format!(
                    r#"tell application "{}" to close active tab of front window"#,
                    browser.name
                ),
            };
            osascript(&script, &[], 10).await?;
            done(
                format!("Closed the tab in {}.", browser.name),
                Some(Share::Browser),
            )
        }
        Command::Page => {
            let browser = current_browser(context)?;
            if browser.id == "safari" {
                let (title, address, text) = crate::apps::safari_page()
                    .await?
                    .ok_or("No page is open in Safari.")?;
                return Ok(Outcome::Page {
                    title: format!("{title} ({address})"),
                    text,
                });
            }
            crate::apps::ensure_public_window(browser).await?;
            let bundle = bundle(browser.id).ok_or("Unknown browser.")?;
            let name = browser.name;
            let page = tokio::task::spawn_blocking(move || {
                if !crate::ax::trusted(true) {
                    return Err(format!("To read pages in {name}, allow Buntini under System Settings → Privacy & Security → Accessibility, then ask again."));
                }
                let pid = crate::ax::pid(bundle).ok_or(format!("{name} isn’t open."))?;
                crate::ax::page_text(pid, 8000).ok_or(format!("I couldn’t read the page in {name}."))
            })
            .await
            .map_err(|_| "The browser couldn’t be read.".to_string())??;
            let (title, text) = page.split_once('\n').unwrap_or(("", &page));
            Ok(Outcome::Page {
                title: title.to_string(),
                text: text.to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(prompt: &str) -> Option<Command> {
        recognise_prompt(prompt)
    }

    #[test]
    fn addresses_keep_paths_query_parameters_and_case() {
        for address in [
            "bbc.co.uk/news",
            "https://example.com/Some/Path?query=A%26B&sort=New#Top",
            "https://example.com/Folder/",
        ] {
            let parsed = reqwest::Url::parse(&if address.contains("://") {
                address.to_string()
            } else {
                format!("https://{address}")
            })
            .unwrap();
            assert_eq!(
                ask(&format!("Hey Buntini, go to {address}")),
                Some(Command::OpenSite {
                    name: address.into(),
                    url: parsed.to_string()
                })
            );
        }
        for prompt in [
            "don't go to https://example.com/private",
            "open http://example.com/path",
            "open https://user:password@example.com/path",
            "open file:///tmp/private",
        ] {
            assert_eq!(ask(prompt), None, "{prompt}");
        }
    }

    #[test]
    fn everyday_browser_requests_are_recognised() {
        let site = |name: &str, url: &str| {
            Some(Command::OpenSite {
                name: name.into(),
                url: url.into(),
            })
        };
        assert_eq!(
            ask("open youtube"),
            site("YouTube", "https://www.youtube.com")
        );
        assert_eq!(
            ask("Hey Buntini, can you go to Reddit for me"),
            site("Reddit", "https://www.reddit.com")
        );
        assert_eq!(
            ask("take me to github.com"),
            site("GitHub", "https://github.com")
        );
        assert_eq!(
            ask("open theverge.com"),
            site("theverge.com", "https://theverge.com")
        );
        assert_eq!(
            ask("open calculator"),
            Some(Command::OpenApp("Calculator".into()))
        );
        for prompt in [
            "google best pizza near me",
            "search youtube for lofi beats",
            "look up the eiffel tower on wikipedia",
            "search the web for flights to porto",
        ] {
            assert_eq!(
                ask(prompt),
                None,
                "Research must enter the integrated workflow: {prompt}"
            );
        }
        assert_eq!(
            ask("Open Google search results for \"C++\" -ads in my browser"),
            Some(Command::Search {
                engine: "google",
                query: "\"C++\" -ads".into()
            })
        );
        assert_eq!(ask("Summarise this page"), Some(Command::Page));
        assert_eq!(ask("what's this article about?"), Some(Command::Page));
        assert_eq!(ask("tl;dr"), Some(Command::Page));
        assert_eq!(ask("close this tab"), Some(Command::CloseTab));
        assert_eq!(ask("what tabs do I have open?"), Some(Command::Tabs));
        assert_eq!(
            ask("switch to my gmail tab"),
            Some(Command::SwitchTab("gmail".into()))
        );
        assert_eq!(ask("open my lease pdf"), None, "files go to the model");
        assert_eq!(
            ask("search for my tax return"),
            None,
            "a plain search could mean files"
        );
        assert_eq!(ask("open the pod bay doors"), None);
        assert_eq!(ask("how do I open a terminal"), None);
        assert_eq!(ask("go to bed early tonight reminder"), None);
    }
}
