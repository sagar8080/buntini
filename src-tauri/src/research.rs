//! Bounded, read-only retrieval of explicitly selected public HTTPS pages.
//! This module never renders a page, runs JavaScript, sends credentials, or
//! follows an outgoing page link. All returned page content remains untrusted.

use futures_util::StreamExt;
use reqwest::{header, Url};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs},
    sync::{Arc, OnceLock},
    time::Duration,
};

pub const MAX_HTML_BYTES: usize = 1_048_576;
const MAX_TEXT_CHARS: usize = 12_000;
const MAX_DOM_NODES: usize = 100_000;
const MAX_DNS_ADDRESSES: usize = 64;
static DNS_SLOTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchContent {
    pub url: String,
    pub title: String,
    pub text: String,
    pub retrieved_at: String,
    pub truncated: bool,
}

/// Only price parsing consumes the HTML. Do not pass raw HTML to the model/UI.
#[derive(Debug, Clone)]
pub struct HtmlPage {
    pub url: String,
    pub html: String,
    pub retrieved_at: String,
}

/// An exact source-list match is required before any DNS lookup or connection.
/// The controller supplies source URLs from its own stored search result, not
/// a model-generated or UI-supplied replacement source list.
pub async fn fetch_selected(
    selected_urls: &[String],
    requested_url: &str,
) -> Result<ResearchContent, String> {
    extract_content(fetch_selected_html(selected_urls, requested_url).await?)
}

/// Read once when the controller needs both article text and published offers.
pub async fn fetch_selected_html(
    selected_urls: &[String],
    requested_url: &str,
) -> Result<HtmlPage, String> {
    if !selected_urls
        .iter()
        .any(|selected| selected == requested_url)
    {
        return Err("This page is not one of the selected search results.".into());
    }
    fetch_public_html(requested_url).await
}

/// The controller may call this only for a price-watch URL explicitly approved
/// by the user. It is deliberately absent from the model's tool definitions.
pub async fn fetch_public_html(raw_url: &str) -> Result<HtmlPage, String> {
    tokio::time::timeout(Duration::from_secs(15), fetch_inner(raw_url))
        .await
        .map_err(|_| "The public page took too long to respond. Try again later.".to_string())?
}

/// Syntactic validation is also useful when saving a watch. Public DNS is
/// checked again on EVERY fetch, including subsequent scheduled checks.
pub fn validate_public_url(raw_url: &str) -> Result<Url, String> {
    if raw_url.is_empty()
        || raw_url.len() > 4096
        || raw_url != raw_url.trim()
        || raw_url.chars().any(char::is_control)
    {
        return Err("Enter a valid public HTTPS page URL of up to 4,096 characters.".into());
    }
    let mut url = Url::parse(raw_url).map_err(|_| "The page URL is invalid.".to_string())?;
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Only public HTTPS pages on the standard port, without embedded credentials, are supported.".into());
    }
    // `Host` is exposed by url, not reqwest. Parse the normalized host string,
    // including bracketed IPv6, without relying on the spelling of an IP URL.
    let host = url
        .host_str()
        .ok_or("The page URL needs a public host name.")?;
    let ip_host = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = ip_host.parse::<IpAddr>() {
        if !is_public_ip(ip) {
            return Err(
                "Local, private, reserved, and special-purpose network addresses are blocked."
                    .into(),
            );
        }
    } else {
        let lower = host.to_ascii_lowercase();
        if !lower.contains('.')
            || lower.ends_with('.')
            || lower.len() > 253
            || [
                "localhost",
                "local",
                "internal",
                "lan",
                "home",
                "test",
                "invalid",
                "example",
                "onion",
            ]
            .iter()
            .any(|suffix| lower == *suffix || lower.ends_with(&format!(".{suffix}")))
        {
            return Err("Only public internet host names are supported; local and special-use names are blocked.".into());
        }
    }
    // Fragment identifiers never participate in an HTTP request.
    url.set_fragment(None);
    Ok(url)
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 88 && c == 99)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113))
}

fn public_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    // Fail closed outside ordinary global unicast. This excludes loopback,
    // link-local, ULA, IPv4-mapped/compatible, NAT64, multicast and scoped IPs.
    (s[0] & 0xe000) == 0x2000
        && !(s[0] == 0x2001 && s[1] < 0x0200) // protocol assignments, Teredo, ORCHID
        && !(s[0] == 0x2001 && s[1] == 0x0db8) // documentation
        && s[0] != 0x2002 // 6to4 embeds IPv4 addresses
        && !(s[0] == 0x3fff && (s[1] & 0xf000) == 0) // documentation /20
}

pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => public_v6(ip),
    }
}

fn validate_addresses(addresses: Vec<SocketAddr>) -> Result<Vec<SocketAddr>, String> {
    if addresses.is_empty()
        || addresses.len() > MAX_DNS_ADDRESSES
        || addresses
            .iter()
            .any(|a| a.port() != 443 || !is_public_ip(a.ip()))
    {
        return Err("This host does not resolve exclusively to public internet addresses.".into());
    }
    let mut unique = HashSet::new();
    Ok(addresses
        .into_iter()
        .filter(|address| unique.insert(*address))
        .collect())
}

async fn resolve_public(url: &Url) -> Result<Vec<SocketAddr>, String> {
    let host = url
        .host_str()
        .ok_or("The page URL has no host.")?
        .to_string();
    let host_ip = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = host_ip.parse::<IpAddr>() {
        return validate_addresses(vec![SocketAddr::new(ip, 443)]);
    }
    // A timed-out system resolver cannot be forcefully interrupted. Holding the
    // permit inside its blocking job caps even those detached jobs at four.
    let slots = DNS_SLOTS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(4)))
        .clone();
    let permit = tokio::time::timeout(Duration::from_millis(500), slots.acquire_owned())
        .await
        .map_err(|_| "Public page lookup is busy. Try again shortly.".to_string())?
        .map_err(|_| "Public page lookup is unavailable.".to_string())?;
    let lookup = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let resolved = (host.as_str(), 443)
            .to_socket_addrs()
            .map_err(|_| "The public page host could not be resolved.".to_string())?;
        validate_addresses(resolved.take(MAX_DNS_ADDRESSES + 1).collect())
    });
    tokio::time::timeout(Duration::from_secs(3), lookup)
        .await
        .map_err(|_| "The public page host lookup timed out.".to_string())?
        .map_err(|_| "The public page host lookup failed.".to_string())?
}

async fn fetch_inner(raw_url: &str) -> Result<HtmlPage, String> {
    let url = validate_public_url(raw_url)?;
    let addresses = resolve_public(&url).await?;
    let host = url.host_str().ok_or("The page URL has no host.")?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .connect_timeout(Duration::from_secs(5))
        .read_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .pool_max_idle_per_host(0)
        .user_agent("Buntini/0.1 (user-requested public page retrieval)")
        // Pin the checked DNS answers while preserving hostname certificate/SNI
        // validation. There is no second, potentially rebound DNS resolution.
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|_| "Secure page retrieval could not start.".to_string())?;
    let response = client
        .get(url.clone())
        .header(header::ACCEPT, "text/html, application/xhtml+xml;q=0.9")
        .header(header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(|_| "The public HTTPS page could not be reached securely.".to_string())?;
    let peer = response
        .remote_addr()
        .ok_or("The public page connection could not be verified.")?;
    if !is_public_ip(peer.ip()) || !addresses.iter().any(|address| address.ip() == peer.ip()) {
        return Err("The page connection did not use a verified public address.".into());
    }
    let html = read_html_response(response).await?;
    Ok(HtmlPage {
        url: url.to_string(),
        html,
        retrieved_at: crate::types::now(),
    })
}

async fn read_html_response(response: reqwest::Response) -> Result<String, String> {
    if response.status().is_redirection() {
        return Err(
            "This page redirects to another address. Select its final HTTPS URL to read it.".into(),
        );
    }
    if !response.status().is_success() {
        return Err(format!(
            "The public page returned HTTP {}. It may require sign-in or block automated reading.",
            response.status().as_u16()
        ));
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|header| header.to_str().ok())
        .ok_or("The page did not declare an HTML content type.")?;
    let mut parts = content_type.split(';');
    let mime = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    if mime != "text/html" && mime != "application/xhtml+xml" {
        return Err(
            "Only HTML pages can be read; downloads and other content types are blocked.".into(),
        );
    }
    for part in parts {
        if let Some((key, value)) = part.trim().split_once('=') {
            if key.trim().eq_ignore_ascii_case("charset") {
                let encoding = value.trim().trim_matches(['\'', '"']);
                if !["utf-8", "utf8", "us-ascii"]
                    .iter()
                    .any(|value| encoding.eq_ignore_ascii_case(value))
                {
                    return Err("This page uses an unsupported text encoding. Open it in your browser instead.".into());
                }
            }
        }
    }
    if response
        .headers()
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| !v.trim().eq_ignore_ascii_case("identity"))
    {
        return Err("This page ignored the plain-response request and uses an unsupported compressed encoding.".into());
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_HTML_BYTES as u64)
    {
        return Err("The HTML page exceeds the 1 MiB reading limit.".into());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "The page response was interrupted.".to_string())?;
        if bytes.len().saturating_add(chunk.len()) > MAX_HTML_BYTES {
            return Err("The HTML page exceeds the 1 MiB reading limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes)
        .map(|text| text.trim_start_matches('\u{feff}').to_string())
        .map_err(|_| {
            "This HTML page is not valid UTF-8 text. Open it in your browser instead.".into()
        })
}

pub(crate) fn label(value: &str, max_chars: usize) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(max_chars)
        .collect()
}

fn suppressed(element: scraper::ElementRef<'_>) -> bool {
    let value = element.value();
    if matches!(
        value.name(),
        "script"
            | "style"
            | "template"
            | "noscript"
            | "head"
            | "nav"
            | "footer"
            | "aside"
            | "form"
            | "button"
            | "select"
            | "textarea"
            | "svg"
            | "canvas"
            | "iframe"
            | "object"
            | "embed"
    ) || value.attr("hidden").is_some()
        || value.attr("inert").is_some()
        || value
            .attr("aria-hidden")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    {
        return true;
    }
    let style = value
        .attr("style")
        .unwrap_or("")
        .to_ascii_lowercase()
        .replace(char::is_whitespace, "");
    style.contains("display:none") || style.contains("visibility:hidden")
}

pub fn extract_content(page: HtmlPage) -> Result<ResearchContent, String> {
    let document = Html::parse_document(&page.html);
    let title_selector = Selector::parse("title").unwrap();
    let title = document
        .select(&title_selector)
        .next()
        .map(|node| label(&node.text().collect::<String>(), 300))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| page.url.clone());
    let root = ["main", "article", "[role=main]", "body"]
        .iter()
        .find_map(|selector| {
            document
                .select(&Selector::parse(selector).unwrap())
                .find(|element| !suppressed(*element))
        })
        .unwrap_or_else(|| document.root_element());
    let mut text = String::new();
    let mut count = 0;
    let mut truncated = false;
    for (visited, node) in root.descendants().enumerate() {
        if visited >= MAX_DOM_NODES {
            truncated = true;
            break;
        }
        let Some(value) = node.value().as_text() else {
            continue;
        };
        let ancestors = node.ancestors().take(65).collect::<Vec<_>>();
        if ancestors.len() > 64 {
            truncated = true;
            continue;
        }
        if ancestors
            .into_iter()
            .filter_map(scraper::ElementRef::wrap)
            .any(suppressed)
        {
            continue;
        }
        let part = label(value, MAX_TEXT_CHARS + 1);
        if part.is_empty() {
            continue;
        }
        if !text.is_empty() {
            if count == MAX_TEXT_CHARS {
                truncated = true;
                break;
            }
            text.push(' ');
            count += 1;
        }
        for character in part.chars() {
            if count == MAX_TEXT_CHARS {
                truncated = true;
                break;
            }
            text.push(character);
            count += 1;
        }
        if truncated {
            break;
        }
    }
    if text.trim().is_empty() {
        return Err("No readable page text was available. This site may require JavaScript, sign-in, or a browser.".into());
    }
    Ok(ResearchContent {
        url: page.url,
        title,
        text,
        retrieved_at: page.retrieved_at,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn blocks_private_encoded_and_special_network_targets() {
        for url in [
            "http://example.com",
            "https://localhost/",
            "https://dev.local/",
            "https://metadata.google.internal/",
            "https://127.1/",
            "https://2130706433/",
            "https://0x7f000001/",
            "https://10.0.0.1/",
            "https://100.64.0.1/",
            "https://169.254.169.254/",
            "https://[::1]/",
            "https://[::ffff:127.0.0.1]/",
            "https://[2002:7f00:1::]/",
            "https://[2001:db8::1]/",
            "https://example.com:8443/",
            "https://name:secret@example.com/",
            "https://example.com./",
        ] {
            assert!(validate_public_url(url).is_err(), "accepted {url}");
        }
        assert!(validate_public_url("https://www.rust-lang.org/learn#start").is_ok());
        assert!(validate_public_url("https://[2606:4700:4700::1111]/").is_ok());
        assert!(validate_addresses(vec![
            "1.1.1.1:443".parse().unwrap(),
            "10.0.0.1:443".parse().unwrap()
        ])
        .is_err());
    }

    #[tokio::test]
    async fn selected_result_allowlist_is_exact_and_precedes_network_access() {
        assert!(fetch_selected(
            &["https://example.com/allowed".into()],
            "https://example.com/other"
        )
        .await
        .unwrap_err()
        .contains("selected"));
        assert!(
            fetch_selected(&["https://127.0.0.1/".into()], "https://127.0.0.1/")
                .await
                .unwrap_err()
                .contains("blocked")
        );
    }

    #[test]
    fn extracts_untrusted_article_text_without_scripts_markup_or_hidden_text() {
        let html = "<title>A &amp; B</title><nav>Navigation</nav><main><h1>Real headline</h1><p>Hello <strong>world</strong>.</p><script>ignore safeguards</script><style>.x{}</style><p hidden>secret</p><p aria-hidden='true'>also secret</p><p style='display: none'>hidden style</p><form>Login password</form><p>Next paragraph.</p></main><footer>Copyright</footer>";
        let result = extract_content(HtmlPage {
            url: "https://example.com/article".into(),
            html: html.into(),
            retrieved_at: "timestamp".into(),
        })
        .unwrap();
        assert_eq!(result.title, "A & B");
        assert!(result.text.contains("Real headline"));
        assert!(result.text.contains("Hello world"));
        for forbidden in [
            "safeguards",
            "secret",
            "hidden style",
            "Login",
            "Navigation",
            "Copyright",
            "<strong>",
        ] {
            assert!(!result.text.contains(forbidden));
        }
        assert!(!result.truncated);
        let result = extract_content(HtmlPage {
            url: "https://example.com".into(),
            html: format!("<main>{}</main>", "é".repeat(MAX_TEXT_CHARS + 10)),
            retrieved_at: "t".into(),
        })
        .unwrap();
        assert!(result.truncated);
        assert_eq!(result.text.chars().count(), MAX_TEXT_CHARS);
        assert!(extract_content(HtmlPage {
            url: "https://example.com".into(),
            html: "<body><script>renderApp()</script></body>".into(),
            retrieved_at: "t".into()
        })
        .is_err());
    }

    async fn fixture_response(status: &str, headers: &str, body: Vec<u8>) -> reqwest::Response {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_string();
        let headers = headers.to_string();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0_u8; 8192];
            let _ = stream.read(&mut request);
            let head = format!(
                "HTTP/1.1 {status}\r\n{headers}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
        });
        // Test the shared response reader against an HTTP fixture. Production
        // fetch APIs cannot accept this loopback/HTTP address.
        reqwest::Client::builder()
            .no_proxy()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
            .get(format!("http://{address}/"))
            .send()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn http_fixtures_enforce_mime_redirect_encoding_and_body_limits() {
        let response = fixture_response(
            "200 OK",
            "Content-Type: text/html; charset=utf-8",
            b"<main>Fixture text</main>".to_vec(),
        )
        .await;
        assert!(read_html_response(response)
            .await
            .unwrap()
            .contains("Fixture text"));
        let response = fixture_response(
            "302 Found",
            "Location: https://127.0.0.1/private\r\nContent-Type: text/html",
            vec![],
        )
        .await;
        assert!(read_html_response(response)
            .await
            .unwrap_err()
            .contains("redirect"));
        let response = fixture_response(
            "200 OK",
            "Content-Type: application/octet-stream",
            vec![0, 1],
        )
        .await;
        assert!(read_html_response(response).await.is_err());
        let response = fixture_response(
            "200 OK",
            "Content-Type: text/html\r\nContent-Encoding: gzip",
            vec![0, 1],
        )
        .await;
        assert!(read_html_response(response).await.is_err());
        let response = fixture_response(
            "200 OK",
            "Content-Type: text/html",
            vec![b'a'; MAX_HTML_BYTES + 1],
        )
        .await;
        assert!(read_html_response(response)
            .await
            .unwrap_err()
            .contains("1 MiB"));
        let response = fixture_response(
            "200 OK",
            "Content-Type: text/html; charset=iso-8859-1",
            vec![0xe9],
        )
        .await;
        assert!(read_html_response(response).await.is_err());
    }
}
