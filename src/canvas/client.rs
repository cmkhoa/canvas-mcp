use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::{Method, Response, StatusCode, header};
use secrecy::ExposeSecret;
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::Semaphore;
use url::Url;

use super::pagination::next_link;
use crate::{
    config::{Auth, Config},
    error::CanvasError,
};

const MAX_ERROR_BODY: usize = 300;
/// Requests in flight at once; Canvas throttles bursts of parallel calls.
const MAX_CONCURRENT_REQUESTS: usize = 5;
/// Hard stop for `get_all`, whatever the item limit.
const MAX_PAGES: usize = 50;
const MAX_PER_PAGE: usize = 100;
/// Canvas's request-cost bucket starts at 700; warn when it runs low.
const LOW_RATE_LIMIT_REMAINING: f64 = 100.0;

/// How throttled or transiently failing requests are retried.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(10),
        }
    }
}

impl RetryPolicy {
    /// Exponential backoff with up to one `base_delay` of jitter.
    fn backoff(&self, attempt: u32) -> Duration {
        let base_ms = self.base_delay.as_millis().max(1) as u64;
        let exp = base_ms.saturating_mul(1u64 << attempt.min(16));
        let jitter = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::from(d.subsec_nanos()) % base_ms);
        Duration::from_millis(exp + jitter).min(self.max_delay)
    }
}

/// Items from a paginated list. `truncated` means Canvas had more than was returned.
#[derive(Debug, Clone)]
pub struct Paged<T> {
    pub items: Vec<T>,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct CanvasClient {
    http: reqwest::Client,
    /// For signed file links, which can point at another host: never carries the token.
    files: reqwest::Client,
    base: Url,
    limiter: Arc<Semaphore>,
    retry: RetryPolicy,
    /// Authenticating with a browser session cookie rather than a token.
    session_mode: bool,
    /// Session-cookie mode only: Canvas requires this header on non-GET requests.
    csrf: Option<header::HeaderValue>,
}

impl CanvasClient {
    pub fn new(config: &Config) -> Result<Self, CanvasError> {
        let sensitive = |value: &str, what: &str| {
            header::HeaderValue::from_str(value)
                .map(|mut v| {
                    v.set_sensitive(true);
                    v
                })
                .map_err(|_| CanvasError::Decode(format!("{what} contains invalid characters")))
        };
        let mut headers = header::HeaderMap::new();
        let mut csrf = None;
        match &config.auth {
            Auth::Token(token) => {
                let value = sensitive(&format!("Bearer {}", token.expose_secret()), "API token")?;
                headers.insert(header::AUTHORIZATION, value);
            }
            Auth::SessionCookie(cookie) => {
                headers.insert(
                    header::COOKIE,
                    sensitive(cookie.expose_secret(), "session cookie")?,
                );
                csrf = csrf_token(cookie.expose_secret())
                    .map(|t| sensitive(&t, "CSRF token"))
                    .transpose()?;
            }
        }
        headers.insert(
            header::ACCEPT,
            header::HeaderValue::from_static("application/json"),
        );

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .user_agent(concat!("canvas-mcp/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            // A redirect to another host would carry the token or cookie with it, and in cookie
            // mode a redirect means Canvas is sending us to its login page.
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let files = reqwest::Client::builder()
            .user_agent(concat!("canvas-mcp/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(120))
            .build()?;
        Ok(Self {
            http,
            files,
            base: config.api_url.clone(),
            limiter: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
            retry: RetryPolicy::default(),
            session_mode: matches!(config.auth, Auth::SessionCookie(_)),
            csrf,
        })
    }

    pub fn with_retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// GET a single Canvas resource. `path` is relative to `/api/v1/`, e.g. `users/self/profile`.
    pub async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, CanvasError> {
        let url = self.url(path)?;
        let resp = self.send(Method::GET, url, query, None::<&()>).await?;
        decode(resp).await
    }

    /// GET a list endpoint, following `rel="next"` links until `limit` items are collected.
    ///
    /// Pass repeated keys (`include[]`) as separate tuples; `per_page` is added here.
    pub async fn get_all<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
        limit: usize,
    ) -> Result<Paged<T>, CanvasError> {
        let limit = limit.max(1);
        let per_page = limit.min(MAX_PER_PAGE).to_string();
        let mut first_query = query.to_vec();
        first_query.push(("per_page", &per_page));

        let mut url = self.url(path)?;
        let mut items = Vec::new();
        for page in 0..MAX_PAGES {
            // Next links already carry the full query string.
            let query: &[(&str, &str)] = if page == 0 { &first_query } else { &[] };
            let resp = self.send(Method::GET, url, query, None::<&()>).await?;
            let next = next_link(resp.headers());
            items.extend(decode::<Vec<T>>(resp).await?);

            if items.len() >= limit {
                let truncated = items.len() > limit || next.is_some();
                items.truncate(limit);
                return Ok(Paged { items, truncated });
            }
            match next {
                Some(next) => url = self.same_origin(&next)?,
                None => {
                    return Ok(Paged {
                        items,
                        truncated: false,
                    });
                }
            }
        }
        tracing::warn!(path, pages = MAX_PAGES, "stopped paginating at page cap");
        Ok(Paged {
            items,
            truncated: true,
        })
    }

    pub async fn post<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, CanvasError> {
        let url = self.url(path)?;
        decode(self.send(Method::POST, url, &[], Some(body)).await?).await
    }

    pub async fn put<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, CanvasError> {
        let url = self.url(path)?;
        decode(self.send(Method::PUT, url, &[], Some(body)).await?).await
    }

    /// Downloads a signed Canvas file link (no bearer token), refusing anything over `max_bytes`.
    pub async fn download(&self, link: &str, max_bytes: u64) -> Result<Vec<u8>, CanvasError> {
        let url =
            Url::parse(link).map_err(|e| CanvasError::Decode(format!("bad file link: {e}")))?;
        if url.scheme() != "https" && url.origin() != self.base.origin() {
            return Err(CanvasError::Decode("file link is not https".into()));
        }
        let _permit = self
            .limiter
            .acquire()
            .await
            .expect("request semaphore is never closed");
        let mut resp = self.files.get(url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(map_status(status, resp.text().await.unwrap_or_default()));
        }
        if resp.content_length().is_some_and(|len| len > max_bytes) {
            return Err(too_large(max_bytes));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = resp.chunk().await? {
            if bytes.len() as u64 + chunk.len() as u64 > max_bytes {
                return Err(too_large(max_bytes));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    fn url(&self, path: &str) -> Result<Url, CanvasError> {
        self.base
            .join(path.trim_start_matches('/'))
            .map_err(|e| CanvasError::Decode(format!("bad path {path:?}: {e}")))
    }

    /// Pagination links come from Canvas; never send the token anywhere else.
    fn same_origin(&self, link: &str) -> Result<Url, CanvasError> {
        let url =
            Url::parse(link).map_err(|e| CanvasError::Decode(format!("bad next link: {e}")))?;
        if url.origin() != self.base.origin() {
            return Err(CanvasError::Decode(format!(
                "pagination link points to another host: {}",
                url.origin().ascii_serialization()
            )));
        }
        Ok(url)
    }

    /// Sends one request, retrying throttling for any method and transient failures for GET.
    async fn send<B: Serialize + ?Sized>(
        &self,
        method: Method,
        url: Url,
        query: &[(&str, &str)],
        body: Option<&B>,
    ) -> Result<Response, CanvasError> {
        let idempotent = method == Method::GET;
        let mut attempt = 0;
        loop {
            let result = {
                let _permit = self
                    .limiter
                    .acquire()
                    .await
                    .expect("request semaphore is never closed");
                let mut req = self.http.request(method.clone(), url.clone()).query(query);
                if let Some(body) = body {
                    req = req.json(body);
                }
                if method != Method::GET
                    && let Some(csrf) = &self.csrf
                {
                    req = req.header("X-CSRF-Token", csrf.clone());
                }
                req.send().await
            };

            let (err, retryable, retry_after) = match result {
                Ok(resp) if resp.status().is_success() => {
                    warn_if_rate_limit_low(&resp);
                    return Ok(resp);
                }
                Ok(resp) => {
                    let status = resp.status();
                    let retry_after = retry_after(resp.headers());
                    // In cookie mode, 401 or a redirect (to the login page) means the session ended.
                    let err = if self.session_mode
                        && (status == StatusCode::UNAUTHORIZED || status.is_redirection())
                    {
                        CanvasError::SessionExpired
                    } else {
                        map_status(status, resp.text().await.unwrap_or_default())
                    };
                    // Canvas rejects throttled requests before acting, so even writes are safe to retry.
                    let retryable = matches!(err, CanvasError::Throttled)
                        || (idempotent
                            && matches!(
                                status,
                                StatusCode::BAD_GATEWAY
                                    | StatusCode::SERVICE_UNAVAILABLE
                                    | StatusCode::GATEWAY_TIMEOUT
                            ));
                    (err, retryable, retry_after)
                }
                Err(e) => {
                    let retryable = idempotent && (e.is_timeout() || e.is_connect());
                    (CanvasError::Network(e), retryable, None)
                }
            };

            if !retryable || attempt >= self.retry.max_retries {
                return Err(err);
            }
            let delay = retry_after.map_or_else(
                || self.retry.backoff(attempt),
                |d| d.min(self.retry.max_delay),
            );
            tracing::warn!(%method, path = url.path(), attempt, ?delay, error = %err, "retrying Canvas request");
            tokio::time::sleep(delay).await;
            attempt += 1;
        }
    }
}

fn too_large(max_bytes: u64) -> CanvasError {
    CanvasError::Decode(format!(
        "file is larger than the {:.1} MB limit",
        max_bytes as f64 / 1_048_576.0
    ))
}

async fn decode<T: DeserializeOwned>(resp: Response) -> Result<T, CanvasError> {
    let bytes = resp.bytes().await?;
    // Canvas prefixes JSON with `while(1);` for cookie-authenticated requests (an old
    // JSON-hijacking guard).
    let json = bytes.strip_prefix(b"while(1);").unwrap_or(&bytes);
    serde_json::from_slice(json).map_err(|e| CanvasError::Decode(e.to_string()))
}

/// The `_csrf_token` cookie value, percent-decoded, which Canvas expects back in
/// `X-CSRF-Token` on writes made with a session cookie.
fn csrf_token(cookie: &str) -> Option<String> {
    cookie.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == "_csrf_token").then(|| {
            percent_encoding::percent_decode_str(value)
                .decode_utf8_lossy()
                .into_owned()
        })
    })
}

fn retry_after(headers: &header::HeaderMap) -> Option<Duration> {
    let secs: u64 = headers
        .get(header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(secs))
}

fn warn_if_rate_limit_low(resp: &Response) {
    let remaining = resp
        .headers()
        .get("x-rate-limit-remaining")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<f64>().ok());
    if let Some(remaining) = remaining.filter(|r| *r < LOW_RATE_LIMIT_REMAINING) {
        tracing::warn!(remaining, "Canvas rate limit budget is running low");
    }
}

fn map_status(status: StatusCode, body: String) -> CanvasError {
    let mut body = body;
    body.truncate(body.floor_char_boundary(MAX_ERROR_BODY));
    match status {
        StatusCode::UNAUTHORIZED => CanvasError::Unauthorized,
        // Canvas signals throttling with 403, not 429.
        StatusCode::FORBIDDEN if body.contains("Rate Limit Exceeded") => CanvasError::Throttled,
        StatusCode::TOO_MANY_REQUESTS => CanvasError::Throttled,
        StatusCode::FORBIDDEN => CanvasError::Forbidden(body),
        StatusCode::NOT_FOUND => CanvasError::NotFound(body),
        StatusCode::UNPROCESSABLE_ENTITY => CanvasError::Validation(body),
        s => CanvasError::Http {
            status: s.as_u16(),
            body,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csrf_token_is_found_and_decoded() {
        assert_eq!(
            csrf_token("canvas_session=abc; _csrf_token=a%2Bb%3D%3D; log_session_id=x").as_deref(),
            Some("a+b==")
        );
        assert_eq!(csrf_token("canvas_session=abc"), None);
    }

    #[test]
    fn backoff_grows_and_respects_cap() {
        let p = RetryPolicy {
            max_retries: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(1000),
        };
        let first = p.backoff(0);
        assert!(first >= Duration::from_millis(100) && first < Duration::from_millis(200));
        let third = p.backoff(2);
        assert!(third >= Duration::from_millis(400) && third < Duration::from_millis(500));
        assert_eq!(p.backoff(10), Duration::from_millis(1000));
    }
}
