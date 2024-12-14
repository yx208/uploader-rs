use thiserror::Error;

#[derive(Debug, Error)]
pub enum UploadError {
    #[error("IO error: {0}")]
    IOError(#[from] tokio::io::Error),

    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Failed to serialize/deserialize: {0}")]
    SerdeError(#[from] serde_json::Error),

    #[error("Upload not found: {0}")]
    UploadNotFound(String),

    #[error("Invalid state transition: {0}")]
    InvalidState(String),

    #[error("Invalid header name")]
    InvalidHeaderName(#[from] reqwest::header::InvalidHeaderName),

    #[error("Invalid header value")]
    InvalidHeaderValue(#[from] reqwest::header::InvalidHeaderValue),
}

pub type UploadResult<T> = Result<T, UploadError>;
