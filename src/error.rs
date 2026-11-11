use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing environment variable {0}")]
    MissingVar(&'static str),
    #[error(
        "set CANVAS_API_TOKEN, or CANVAS_SESSION_COOKIE if your school doesn't let you create tokens"
    )]
    MissingAuth,
    #[error("set only one of CANVAS_API_TOKEN and CANVAS_SESSION_COOKIE")]
    ConflictingAuth,
    #[error(
        "CANVAS_SESSION_COOKIE doesn't look like a Cookie header (expected name=value pairs on one line)"
    )]
    InvalidCookie,
    #[error("invalid CANVAS_API_URL: {0}")]
    InvalidUrl(String),
    #[error("invalid CANVAS_TIMEZONE {0:?}: use an IANA name like America/Chicago")]
    InvalidTimezone(String),
}

/// Canvas failures, worded for the model: each message says what went wrong and what to do.
#[derive(Debug, Error)]
pub enum CanvasError {
    #[error(
        "Canvas rejected the API token (401). It may be expired or revoked; generate a new one."
    )]
    Unauthorized,
    #[error(
        "Your Canvas login session has ended (signed out or expired). Copy a fresh Cookie header from your browser into CANVAS_SESSION_COOKIE and restart."
    )]
    SessionExpired,
    #[error("Canvas denied access (403): {0}")]
    Forbidden(String),
    #[error("Canvas rate limit exceeded; wait a moment and retry.")]
    Throttled,
    #[error("Not found in Canvas (404): {0}")]
    NotFound(String),
    #[error("Canvas rejected the request (422): {0}")]
    Validation(String),
    #[error("Canvas returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("could not reach Canvas: {0}")]
    Network(#[from] reqwest::Error),
    #[error("unexpected response from Canvas: {0}")]
    Decode(String),
}

impl CanvasError {
    /// The HTTP status behind this error, when there was one.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Unauthorized | Self::SessionExpired => Some(401),
            Self::Forbidden(_) => Some(403),
            Self::Throttled => Some(429),
            Self::NotFound(_) => Some(404),
            Self::Validation(_) => Some(422),
            Self::Http { status, .. } => Some(*status),
            Self::Network(_) | Self::Decode(_) => None,
        }
    }
}
