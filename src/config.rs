use secrecy::SecretString;
use url::Url;

use crate::error::ConfigError;

#[derive(Debug, Clone)]
pub struct Config {
    /// Canvas REST base, always ending in `/api/v1/`.
    pub api_url: Url,
    pub token: SecretString,
}

impl Config {
    pub fn new(api_url: &str, token: impl Into<String>) -> Result<Self, ConfigError> {
        let mut url =
            Url::parse(api_url.trim()).map_err(|e| ConfigError::InvalidUrl(e.to_string()))?;

        let is_local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if url.scheme() != "https" && !is_local {
            return Err(ConfigError::InvalidUrl(
                "CANVAS_API_URL must use https".into(),
            ));
        }
        if !url.path().trim_end_matches('/').ends_with("/api/v1") {
            return Err(ConfigError::InvalidUrl(
                "CANVAS_API_URL must end in /api/v1 (e.g. https://school.instructure.com/api/v1)"
                    .into(),
            ));
        }
        // Url::join drops the last path segment unless the base ends in '/'.
        if !url.path().ends_with('/') {
            url.set_path(&format!("{}/", url.path()));
        }

        let token = token.into();
        if token.trim().is_empty() {
            return Err(ConfigError::MissingVar("CANVAS_API_TOKEN"));
        }
        Ok(Self {
            api_url: url,
            token: SecretString::from(token.trim().to_owned()),
        })
    }

    /// Reads `CANVAS_API_URL` and `CANVAS_API_TOKEN`, loading a `.env` file if present.
    pub fn from_env() -> Result<Self, ConfigError> {
        dotenvy::dotenv().ok();
        let url = std::env::var("CANVAS_API_URL")
            .map_err(|_| ConfigError::MissingVar("CANVAS_API_URL"))?;
        let token = std::env::var("CANVAS_API_TOKEN")
            .map_err(|_| ConfigError::MissingVar("CANVAS_API_TOKEN"))?;
        Self::new(&url, token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_trailing_slash() {
        let c = Config::new("https://x.instructure.com/api/v1", "t").unwrap();
        assert_eq!(c.api_url.as_str(), "https://x.instructure.com/api/v1/");
    }

    #[test]
    fn rejects_http_and_missing_api_path() {
        assert!(Config::new("http://x.instructure.com/api/v1", "t").is_err());
        assert!(Config::new("https://x.instructure.com", "t").is_err());
        assert!(Config::new("https://x.instructure.com/api/v1", " ").is_err());
    }
}
