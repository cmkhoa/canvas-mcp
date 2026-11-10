use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing environment variable {0}")]
    MissingVar(&'static str),
    #[error("invalid CANVAS_API_URL: {0}")]
    InvalidUrl(String),
}

/// Canvas failures, worded for the model: each message says what went wrong and what to do.
#[derive(Debug, Error)]
pub enum CanvasError {
    #[error(
        "Canvas rejected the API token (401). It may be expired or revoked; generate a new one."
    )]
    Unauthorized,
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
