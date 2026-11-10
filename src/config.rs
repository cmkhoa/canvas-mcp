use chrono_tz::Tz;
use secrecy::SecretString;
use url::Url;

use crate::error::ConfigError;

#[derive(Debug, Clone)]
pub struct Config {
    /// Canvas REST base, always ending in `/api/v1/`.
    pub api_url: Url,
    pub token: SecretString,
    /// Zone used when showing dates to the model.
    pub timezone: Tz,
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
            timezone: Tz::UTC,
        })
    }

    pub fn with_timezone(mut self, timezone: Tz) -> Self {
        self.timezone = timezone;
        self
    }

    /// Reads `CANVAS_API_URL`, `CANVAS_API_TOKEN` and optional `CANVAS_TIMEZONE`
    /// (default: this computer's zone), loading a `.env` file if present.
    pub fn from_env() -> Result<Self, ConfigError> {
        dotenvy::dotenv().ok();
        let url = std::env::var("CANVAS_API_URL")
            .map_err(|_| ConfigError::MissingVar("CANVAS_API_URL"))?;
        let token = std::env::var("CANVAS_API_TOKEN")
            .map_err(|_| ConfigError::MissingVar("CANVAS_API_TOKEN"))?;
        let timezone = match std::env::var("CANVAS_TIMEZONE") {
            Ok(name) if !name.trim().is_empty() => parse_timezone(&name)?,
            _ => system_timezone(),
        };
        Ok(Self::new(&url, token)?.with_timezone(timezone))
    }
}

/// Parses an IANA zone name such as `America/Chicago`.
pub fn parse_timezone(name: &str) -> Result<Tz, ConfigError> {
    name.trim()
        .parse()
        .map_err(|_| ConfigError::InvalidTimezone(name.trim().to_owned()))
}

fn system_timezone() -> Tz {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|name| name.parse().ok())
        .unwrap_or(Tz::UTC)
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
    fn parses_iana_timezones() {
        assert_eq!(
            parse_timezone(" America/Chicago ").unwrap(),
            Tz::America__Chicago
        );
        assert!(parse_timezone("Mars/Olympus_Mons").is_err());
    }

    #[test]
    fn rejects_http_and_missing_api_path() {
        assert!(Config::new("http://x.instructure.com/api/v1", "t").is_err());
        assert!(Config::new("https://x.instructure.com", "t").is_err());
        assert!(Config::new("https://x.instructure.com/api/v1", " ").is_err());
    }
}
