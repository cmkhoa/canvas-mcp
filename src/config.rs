use chrono_tz::Tz;
use secrecy::SecretString;
use url::Url;

use crate::error::ConfigError;

/// How requests prove who the user is.
#[derive(Debug, Clone)]
pub enum Auth {
    /// A personal access token, sent as `Authorization: Bearer`.
    Token(SecretString),
    /// The browser's `Cookie` header for the Canvas site, for schools that don't let students
    /// create tokens. As powerful as being signed in; expires when the browser session does.
    SessionCookie(SecretString),
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Canvas REST base, always ending in `/api/v1/`.
    pub api_url: Url,
    pub auth: Auth,
    /// Zone used when showing dates to the model.
    pub timezone: Tz,
}

impl Config {
    /// Configuration authenticated with a personal access token.
    pub fn new(api_url: &str, token: impl Into<String>) -> Result<Self, ConfigError> {
        let token = token.into();
        if token.trim().is_empty() {
            return Err(ConfigError::MissingAuth);
        }
        Self::build(
            api_url,
            Auth::Token(SecretString::from(token.trim().to_owned())),
        )
    }

    /// Configuration authenticated with the browser's Canvas `Cookie` header.
    pub fn with_session_cookie(
        api_url: &str,
        cookie: impl Into<String>,
    ) -> Result<Self, ConfigError> {
        let cookie = cookie.into();
        // Accept a pasted "Cookie: …" header line as well as just its value.
        let value = cookie.trim();
        let value = value
            .strip_prefix("Cookie:")
            .or_else(|| value.strip_prefix("cookie:"))
            .unwrap_or(value)
            .trim();
        if value.is_empty() || !value.contains('=') || value.contains(['\r', '\n']) {
            return Err(ConfigError::InvalidCookie);
        }
        Self::build(
            api_url,
            Auth::SessionCookie(SecretString::from(value.to_owned())),
        )
    }

    fn build(api_url: &str, auth: Auth) -> Result<Self, ConfigError> {
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

        Ok(Self {
            api_url: url,
            auth,
            timezone: Tz::UTC,
        })
    }

    pub fn with_timezone(mut self, timezone: Tz) -> Self {
        self.timezone = timezone;
        self
    }

    /// Reads `CANVAS_API_URL`, exactly one of `CANVAS_API_TOKEN` / `CANVAS_SESSION_COOKIE`,
    /// and optional `CANVAS_TIMEZONE` (default: this computer's zone), loading `.env` if present.
    pub fn from_env() -> Result<Self, ConfigError> {
        dotenvy::dotenv().ok();
        let url = std::env::var("CANVAS_API_URL")
            .map_err(|_| ConfigError::MissingVar("CANVAS_API_URL"))?;
        let non_empty = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let timezone = match std::env::var("CANVAS_TIMEZONE") {
            Ok(name) if !name.trim().is_empty() => parse_timezone(&name)?,
            _ => system_timezone(),
        };
        let config = match (
            non_empty("CANVAS_API_TOKEN"),
            non_empty("CANVAS_SESSION_COOKIE"),
        ) {
            (Some(token), None) => Self::new(&url, token)?,
            (None, Some(cookie)) => Self::with_session_cookie(&url, cookie)?,
            (Some(_), Some(_)) => return Err(ConfigError::ConflictingAuth),
            (None, None) => return Err(ConfigError::MissingAuth),
        };
        Ok(config.with_timezone(timezone))
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
    fn session_cookie_accepts_pasted_header_lines() {
        let c = Config::with_session_cookie(
            "https://x.instructure.com/api/v1",
            " Cookie: canvas_session=abc; _csrf_token=x%2By ",
        )
        .unwrap();
        let Auth::SessionCookie(cookie) = c.auth else {
            panic!("expected cookie auth")
        };
        use secrecy::ExposeSecret;
        assert_eq!(
            cookie.expose_secret(),
            "canvas_session=abc; _csrf_token=x%2By"
        );
    }

    #[test]
    fn session_cookie_rejects_junk_and_header_injection() {
        let url = "https://x.instructure.com/api/v1";
        assert!(Config::with_session_cookie(url, "").is_err());
        assert!(Config::with_session_cookie(url, "not a cookie").is_err());
        assert!(Config::with_session_cookie(url, "a=b\r\nX-Evil: 1").is_err());
    }

    #[test]
    fn rejects_http_and_missing_api_path() {
        assert!(Config::new("http://x.instructure.com/api/v1", "t").is_err());
        assert!(Config::new("https://x.instructure.com", "t").is_err());
        assert!(Config::new("https://x.instructure.com/api/v1", " ").is_err());
    }
}
