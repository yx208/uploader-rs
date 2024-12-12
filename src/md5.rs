use std::path::PathBuf;
use thiserror::Error;
use anyhow::Result;

#[derive(Error, Debug)]
pub enum MD5Error {
    #[error("IO error: {0}")]
    IOError(#[from] tokio::io::Error),

    #[error("Invalid path: {0}")]
    InvalidPath(String),
}

pub type MD5Result<T> = Result<T, MD5Error>;

pub struct MDCalculateResult {
    pub hash: String,
    pub file_size: u64,
}

pub struct MD5Calculator {
    file_path: PathBuf,
}

impl MD5Calculator {
    pub fn new(file_path: PathBuf) -> Self {
        Self { file_path }
    }
}
