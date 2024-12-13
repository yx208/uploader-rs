use thiserror::Error;

#[derive(Debug, Error)]
pub enum UploadError {
    #[error("IO error: {0}")]
    IOError(#[from] tokio::io::Error),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Failed to serialize/deserialize: {0}")]
    SerdeError(#[from] serde_json::Error),
}

pub type UploadResult<T> = Result<T, UploadError>;
