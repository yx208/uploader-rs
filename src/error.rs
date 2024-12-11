use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TusError {
    #[error("IO error: {0}")]
    IOError(#[from] io::Error),

    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("Failed to serialize/deserialize: {0}")]
    SerdeError(#[from] serde_json::Error),

    #[error("Upload not found: {0}")]
    UploadNotFound(String),

    #[error("Invalid state transition: {0}")]
    InvalidState(String),

    #[error("Configuration error: {0}")]
    Config(String),
}

pub type TusResult<T> = Result<T, TusError>;
