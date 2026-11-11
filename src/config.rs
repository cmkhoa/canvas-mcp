use std::path::{Path, PathBuf};

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
    /// The `.env` file settings were read from, if any.
    pub settings_file: Option<PathBuf>,
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
            Auth::SessionCookie(SecretString::from(keep_canvas_cookies(value))),
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
            settings_file: None,
        })
    }

    pub fn with_timezone(mut self, timezone: Tz) -> Self {
        self.timezone = timezone;
        self
    }

    /// Reads `CANVAS_API_URL`, exactly one of `CANVAS_API_TOKEN` / `CANVAS_SESSION_COOKIE`,
    /// and optional `CANVAS_TIMEZONE` (default: this computer's zone), loading `.env` if present.
    ///
    /// Settings come from the first of: `env_file` (`--env-file` / `CANVAS_MCP_ENV_FILE`),
    /// `./.env`, or `~/.config/canvas-mcp/.env`. Variables already set in the environment win.
    pub fn from_env(env_file: Option<&Path>) -> Result<Self, ConfigError> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
        let cwd = std::env::current_dir().unwrap_or_default();
        let settings_file = find_env_file(env_file, &cwd, home.as_deref(), xdg.as_deref())?;
        if let Some(path) = &settings_file {
            load_dotenv(path)?;
        }
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
        Ok(Self {
            settings_file,
            ..config.with_timezone(timezone)
        })
    }
}

/// The settings file to read: the explicit one (which must exist), else `cwd/.env`, else
/// `$XDG_CONFIG_HOME/canvas-mcp/.env` or `~/.config/canvas-mcp/.env`. Only one file is read,
/// so a token in one and a cookie in another can't mix.
pub fn find_env_file(
    explicit: Option<&Path>,
    cwd: &Path,
    home: Option<&Path>,
    xdg_config: Option<&Path>,
) -> Result<Option<PathBuf>, ConfigError> {
    if let Some(path) = explicit {
        return if path.is_file() {
            Ok(Some(path.to_path_buf()))
        } else {
            Err(ConfigError::DotEnv(format!(
                "settings file not found: {}",
                path.display()
            )))
        };
    }
    let config_dir = xdg_config
        .map(Path::to_path_buf)
        .or_else(|| home.map(|h| h.join(".config")));
    let candidates = [
        Some(cwd.join(".env")),
        config_dir.map(|d| d.join("canvas-mcp").join(".env")),
    ];
    Ok(candidates.into_iter().flatten().find(|p| p.is_file()))
}

/// Loads a `.env` file. A file that fails to parse is an error (silently skipping it hides the
/// real problem), reported by variable name only, since lines hold secrets.
fn load_dotenv(path: &Path) -> Result<(), ConfigError> {
    match dotenvy::from_path(path) {
        Ok(()) => Ok(()),
        Err(dotenvy::Error::LineParse(line, _)) => {
            let name: String = line
                .split('=')
                .next()
                .unwrap_or("")
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .take(64)
                .collect();
            Err(ConfigError::DotEnv(format!(
                "couldn't read {name} in {}. If its value contains $, spaces or quotes, wrap it in single quotes: {name}='...'",
                path.display()
            )))
        }
        Err(e) => Err(ConfigError::DotEnv(e.to_string())),
    }
}

/// Canvas's own session cookies. When present, everything else in a pasted Cookie header
/// (analytics, consent banners) is dropped: Canvas doesn't need it.
const CANVAS_COOKIES: [&str; 5] = [
    "canvas_session",
    "_normandy_session",
    "_legacy_normandy_session",
    "_csrf_token",
    "log_session_id",
];

fn keep_canvas_cookies(header: &str) -> String {
    let pairs: Vec<&str> = header
        .split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    let name = |pair: &str| pair.split_once('=').map(|(n, _)| n.trim().to_owned());
    let has_canvas_session = pairs.iter().any(|p| {
        matches!(
            name(p).as_deref(),
            Some("canvas_session" | "_normandy_session")
        )
    });
    if !has_canvas_session {
        return pairs.join("; ");
    }
    pairs
        .into_iter()
        .filter(|p| name(p).is_some_and(|n| CANVAS_COOKIES.contains(&n.as_str())))
        .collect::<Vec<_>>()
        .join("; ")
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
    fn env_file_search_order() {
        let dir = tempfile::tempdir().unwrap();
        let (cwd, home) = (dir.path().join("work"), dir.path().join("home"));
        let user_file = home.join(".config/canvas-mcp/.env");
        std::fs::create_dir_all(user_file.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(&user_file, "X=1").unwrap();

        // Nothing in the working directory: the per-user file.
        assert_eq!(
            find_env_file(None, &cwd, Some(&home), None).unwrap(),
            Some(user_file.clone())
        );
        // A project .env wins over it.
        std::fs::write(cwd.join(".env"), "X=2").unwrap();
        assert_eq!(
            find_env_file(None, &cwd, Some(&home), None).unwrap(),
            Some(cwd.join(".env"))
        );
        // An explicit file wins over both, and must exist.
        assert_eq!(
            find_env_file(Some(&user_file), &cwd, Some(&home), None).unwrap(),
            Some(user_file)
        );
        assert!(find_env_file(Some(&cwd.join("missing.env")), &cwd, Some(&home), None).is_err());
        // XDG_CONFIG_HOME replaces ~/.config.
        let xdg = dir.path().join("xdg");
        std::fs::remove_file(cwd.join(".env")).unwrap();
        assert_eq!(
            find_env_file(None, &cwd, Some(&home), Some(&xdg)).unwrap(),
            None
        );
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
    fn session_cookie_drops_analytics_cookies() {
        let c = Config::with_session_cookie(
            "https://x.instructure.com/api/v1",
            "cookie_notice=1; _ga_X=GS2.1.s1$o1$g0; log_session_id=L; canvas_session=S; _csrf_token=T",
        )
        .unwrap();
        let Auth::SessionCookie(cookie) = c.auth else {
            panic!("expected cookie auth")
        };
        use secrecy::ExposeSecret;
        assert_eq!(
            cookie.expose_secret(),
            "log_session_id=L; canvas_session=S; _csrf_token=T"
        );
        // Unfamiliar cookie names (a self-hosted Canvas) are kept as-is.
        assert_eq!(
            keep_canvas_cookies("my_session=a; other=b"),
            "my_session=a; other=b"
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
