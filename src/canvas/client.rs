use std::time::Duration;

use reqwest::{StatusCode, header};
use secrecy::ExposeSecret;
use serde::de::DeserializeOwned;
use url::Url;

use crate::{config::Config, error::CanvasError};

const MAX_ERROR_BODY: usize = 300;

#[derive(Debug, Clone)]
pub struct CanvasClient {
    http: reqwest::Client,
    base: Url,
}

impl CanvasClient {
    pub fn new(config: &Config) -> Result<Self, CanvasError> {
        let mut auth =
            header::HeaderValue::from_str(&format!("Bearer {}", config.token.expose_secret()))
                .map_err(|_| CanvasError::Decode("API token contains invalid characters".into()))?;
        auth.set_sensitive(true);

        let mut headers = header::HeaderMap::new();
        headers.insert(header::AUTHORIZATION, auth);
        headers.insert(
            header::ACCEPT,
            header::HeaderValue::from_static("application/json"),
        );

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .user_agent(concat!("canvas-mcp/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            http,
            base: config.api_url.clone(),
        })
    }

    /// GET a single Canvas resource. `path` is relative to `/api/v1/`, e.g. `users/self/profile`.
    pub async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, CanvasError> {
        let url = self
            .base
            .join(path.trim_start_matches('/'))
            .map_err(|e| CanvasError::Decode(format!("bad path {path:?}: {e}")))?;
        let resp = self.http.get(url).query(query).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(map_status(status, body));
        }
        let bytes = resp.bytes().await?;
        serde_json::from_slice(&bytes).map_err(|e| CanvasError::Decode(e.to_string()))
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
