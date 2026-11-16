//! One request to a service: the budget is reserved first, the call goes on a client that never
//! follows a redirect or retries, the answer is read under a size cap, and the service's
//! rate-limit headers are read on the way.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use reqwest::header::HeaderMap;
use reqwest::RequestBuilder;
use serde_json::Value;

use super::Kind;
use crate::integrations::{cancellable, json_response, USER_AGENT};

/// Notion's API version these requests are written for.
const NOTION_VERSION: &str = "2022-06-28";
/// Cal.com's bookings API version.
const CALCOM_VERSION: &str = "2024-08-13";

#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    /// The key was refused. Polling stops until it changes.
    Rejected(String),
    /// The service asked to wait until this Unix time; 0 when it said no time.
    Limited { until: i64, message: String },
    /// Buntini's own daily or monthly allowance for the service is used up.
    Budget(String),
    /// Unreachable, unreadable, or an error in the answer.
    Other(String),
    Cancelled,
}

impl Failure {
    pub fn message(&self) -> &str {
        match self {
            Failure::Rejected(message) | Failure::Budget(message) | Failure::Other(message) => message,
            Failure::Limited { message, .. } => message,
            Failure::Cancelled => "Cancelled.",
        }
    }

    pub fn unreadable(kind: Kind) -> Self {
        Failure::Other(format!("{} returned an unreadable response.", kind.name()))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RateLimit {
    pub remaining: Option<u64>,
    /// When the allowance refills, Unix seconds.
    pub reset_at: Option<i64>,
    /// When the service said to ask again, Unix seconds.
    pub retry_at: Option<i64>,
}

impl RateLimit {
    /// Reads `x-ratelimit-*` (GitHub, Vercel), `ratelimit-*` (GitLab, Resend), the structured
    /// `ratelimit: "policy";r=…;t=…` (Gitea, Forgejo) and `retry-after`. A reset is a Unix time
    /// in seconds or milliseconds, or seconds from now.
    pub fn read(headers: &HeaderMap, now: i64) -> Self {
        let header = |names: &[&str]| {
            names.iter().find_map(|name| headers.get(*name)?.to_str().ok().map(|value| value.trim().to_string()))
        };
        let moment = |raw: String| -> Option<i64> {
            if let Ok(number) = raw.parse::<f64>() {
                if !number.is_finite() || number < 0.0 {
                    return None;
                }
                let number = number.ceil() as i64;
                return Some(match number {
                    n if n > 100_000_000_000 => n / 1000,
                    n if n > 1_000_000_000 => n,
                    n => now + n,
                });
            }
            chrono::DateTime::parse_from_rfc2822(&raw).ok().map(|date| date.timestamp())
        };
        let structured = |key: &str| -> Option<String> {
            let value = header(&["ratelimit"])?;
            value.split(';').find_map(|part| part.trim().strip_prefix(key)?.strip_prefix('=').map(str::to_string))
        };
        Self {
            remaining: header(&["x-ratelimit-remaining", "ratelimit-remaining"]).or_else(|| structured("r")).and_then(|value| value.parse().ok()),
            reset_at: header(&["x-ratelimit-reset", "ratelimit-reset"]).or_else(|| structured("t")).and_then(moment),
            retry_at: header(&["retry-after"]).and_then(moment),
        }
    }

    /// The allowance is all but spent: the next call waits for the reset.
    pub fn exhausted(&self) -> bool {
        self.remaining.is_some_and(|remaining| remaining <= 2)
    }

    /// When asking again is fine, if the service said to wait.
    pub fn wait_until(&self, now: i64) -> Option<i64> {
        let reset = if self.exhausted() { self.reset_at } else { None };
        [self.retry_at, reset].into_iter().flatten().filter(|at| *at > now).max()
    }
}

/// What a refused answer means for the poller.
pub fn refused(kind: Kind, status: u16, limit: &RateLimit, message: String, now: i64) -> Failure {
    let until = limit.wait_until(now).unwrap_or(0);
    match status {
        // Resend answers a key it doesn't know with 400; every request here is fixed, so a 400
        // can only be the key.
        400 if kind == Kind::Resend => Failure::Rejected(format!("{} rejected the saved credentials. Check them in Settings.", kind.name())),
        429 => Failure::Limited { until, message },
        // GitHub answers 403 for its secondary limits, with retry-after or nothing remaining.
        403 if limit.retry_at.is_some() || limit.remaining == Some(0) => Failure::Limited { until, message },
        401 | 403 => Failure::Rejected(message),
        _ => Failure::Other(message),
    }
}

/// Reads an answer: its rate-limit headers, then its JSON under `cap` bytes.
pub async fn read(response: Result<reqwest::Response, reqwest::Error>, kind: Kind, cap: usize, now: i64) -> (Result<Value, Failure>, RateLimit) {
    let (status, limit) = match &response {
        Ok(response) => (response.status().as_u16(), RateLimit::read(response.headers(), now)),
        Err(_) => (0, RateLimit::default()),
    };
    let value = json_response(response, kind.name(), cap).await.map_err(|message| {
        if (200..300).contains(&status) || status == 0 {
            Failure::Other(message)
        } else {
            refused(kind, status, &limit, message, now)
        }
    });
    (value, limit)
}

type Reserve = Box<dyn Fn() -> Result<(), String> + Send + Sync>;

/// The connection to one service for one poll.
pub struct Net {
    pub kind: Kind,
    client: reqwest::Client,
    key: String,
    /// `https://host[:port]`.
    base: reqwest::Url,
    reserve: Reserve,
    cancel: Arc<AtomicBool>,
    limit: Mutex<RateLimit>,
}

impl Net {
    pub fn new(kind: Kind, key: String, host: &str, reserve: Reserve, cancel: Arc<AtomicBool>) -> Result<Self, Failure> {
        let client = crate::web::private_client_builder()
            .timeout(std::time::Duration::from_secs(20))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|_| Failure::Other("Cannot initialize a secure connection.".into()))?;
        let base = reqwest::Url::parse(&format!("https://{host}/")).map_err(|_| Failure::Other(format!("{}'s address isn't valid.", kind.name())))?;
        Ok(Self { kind, client, key, base, reserve, cancel, limit: Mutex::new(RateLimit::default()) })
    }

    /// `https://host/a/b`, each segment escaped.
    pub fn url(&self, segments: &[&str]) -> reqwest::Url {
        let mut url = self.base.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.clear().extend(segments);
        }
        url
    }

    pub async fn get(&self, segments: &[&str], query: &[(&str, String)], cap: usize) -> Result<Value, Failure> {
        let request = self.authorize(self.client.get(self.url(segments)).query(query));
        self.send(request, cap).await
    }

    /// A GET without the key, for a public page on the same host.
    pub async fn get_public(&self, segments: &[&str], cap: usize) -> Result<Value, Failure> {
        self.send(self.client.get(self.url(segments)).header("Accept", "application/json"), cap).await
    }

    /// A POST that only reads: a GraphQL query or a search.
    pub async fn post(&self, segments: &[&str], body: &Value, cap: usize) -> Result<Value, Failure> {
        let request = self.authorize(self.client.post(self.url(segments)).json(body));
        self.send(request, cap).await
    }

    /// The rate limit the last answer reported.
    pub fn limit(&self) -> RateLimit {
        *self.limit.lock().unwrap()
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn authorize(&self, request: RequestBuilder) -> RequestBuilder {
        let request = request.header("Accept", "application/json");
        match self.kind {
            Kind::Gitea => request.header("Authorization", format!("token {}", self.key)),
            Kind::N8n => request.header("X-N8N-API-KEY", &self.key),
            Kind::Notion => request.bearer_auth(&self.key).header("Notion-Version", NOTION_VERSION),
            Kind::Calcom => request.bearer_auth(&self.key).header("cal-api-version", CALCOM_VERSION),
            _ => request.bearer_auth(&self.key),
        }
    }

    async fn send(&self, request: RequestBuilder, cap: usize) -> Result<Value, Failure> {
        if self.cancelled() {
            return Err(Failure::Cancelled);
        }
        (self.reserve)().map_err(Failure::Budget)?;
        let now = chrono::Utc::now().timestamp();
        let kind = self.kind;
        let work = async move { Ok(read(request.send().await, kind, cap, now).await) };
        let (value, limit) = cancellable(work, self.cancel.clone()).await.map_err(|_| Failure::Cancelled)?;
        *self.limit.lock().unwrap() = limit;
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderName, HeaderValue};

    const NOW: i64 = 1_794_820_320;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(HeaderName::from_bytes(name.as_bytes()).unwrap(), HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn rate_limits_read_in_each_services_dialect() {
        // GitHub and Vercel: a Unix reset.
        let github = RateLimit::read(&headers(&[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "1794822000")]), NOW);
        assert_eq!(github, RateLimit { remaining: Some(0), reset_at: Some(1_794_822_000), retry_at: None });
        assert_eq!(github.wait_until(NOW), Some(1_794_822_000));
        // Resend: seconds from now.
        let resend = RateLimit::read(&headers(&[("ratelimit-remaining", "1"), ("ratelimit-reset", "1")]), NOW);
        assert_eq!(resend.wait_until(NOW), Some(NOW + 1));
        // Milliseconds and an HTTP date.
        let millis = RateLimit::read(&headers(&[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "1794822000000")]), NOW);
        assert_eq!(millis.reset_at, Some(1_794_822_000));
        let dated = RateLimit::read(&headers(&[("retry-after", "Mon, 16 Nov 2026 09:14:00 GMT")]), NOW);
        assert_eq!(dated.retry_at, Some(NOW + 120));
        // Plenty left: no waiting for the reset.
        let plenty = RateLimit::read(&headers(&[("x-ratelimit-remaining", "4990"), ("x-ratelimit-reset", "1794822000")]), NOW);
        assert_eq!(plenty.wait_until(NOW), None);
        // Gitea and Forgejo: the structured header, seconds to the reset.
        let gitea = RateLimit::read(&headers(&[("ratelimit", "\"baseline\";r=0;t=600")]), NOW);
        assert_eq!(gitea, RateLimit { remaining: Some(0), reset_at: Some(NOW + 600), retry_at: None });
        // Nonsense is ignored.
        let odd = RateLimit::read(&headers(&[("x-ratelimit-remaining", "lots"), ("retry-after", "-5")]), NOW);
        assert_eq!(odd, RateLimit::default());
    }

    #[test]
    fn refusals_become_waits_or_a_stopped_key() {
        let none = RateLimit::default();
        let github = Kind::Github;
        assert_eq!(refused(github, 401, &none, "no".into(), NOW), Failure::Rejected("no".into()));
        assert_eq!(refused(github, 403, &none, "no".into(), NOW), Failure::Rejected("no".into()));
        let secondary = RateLimit { retry_at: Some(NOW + 60), ..Default::default() };
        assert_eq!(refused(github, 403, &secondary, "slow".into(), NOW), Failure::Limited { until: NOW + 60, message: "slow".into() });
        assert_eq!(refused(github, 429, &none, "slow".into(), NOW), Failure::Limited { until: 0, message: "slow".into() });
        assert_eq!(refused(github, 502, &none, "down".into(), NOW), Failure::Other("down".into()));
        assert_eq!(refused(github, 400, &none, "odd".into(), NOW), Failure::Other("odd".into()));
        assert!(matches!(refused(Kind::Resend, 400, &none, "odd".into(), NOW), Failure::Rejected(_)));
    }

    #[test]
    fn paths_are_escaped_one_segment_at_a_time() {
        let net = Net::new(Kind::Gitea, "k".into(), "git.example.com:3000", Box::new(|| Ok(())), Arc::default()).unwrap();
        let url = net.url(&["api", "v1", "repos", "me", "a/b?c", "pulls", "4"]);
        assert_eq!(url.as_str(), "https://git.example.com:3000/api/v1/repos/me/a%2Fb%3Fc/pulls/4");
        assert!(Net::new(Kind::Gitea, "k".into(), "bad host", Box::new(|| Ok(())), Arc::default()).is_err());
    }

    #[tokio::test]
    async fn nothing_is_sent_once_the_budget_or_the_person_says_stop() {
        let net = Net::new(Kind::Github, "k".into(), "api.github.com", Box::new(|| Err("Today’s limit is used up.".into())), Arc::default()).unwrap();
        assert_eq!(net.get(&["user"], &[], 1024).await, Err(Failure::Budget("Today’s limit is used up.".into())));
        let cancel = Arc::new(AtomicBool::new(true));
        let net = Net::new(Kind::Github, "k".into(), "api.github.com", Box::new(|| panic!("reserved after cancel")), cancel).unwrap();
        assert_eq!(net.get(&["user"], &[], 1024).await, Err(Failure::Cancelled));
    }
}
