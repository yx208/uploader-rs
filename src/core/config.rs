use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use crate::core::error::{UploadError, UploadResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TusConfig {
    /// 服务基础 url
    pub endpoint: String,

    /// 额外的请求头参数
    pub headers: HashMap<String, String>,

    /// 最大同时上传任务
    pub max_concurrent: u8,

    /// 每次上传块大小
    pub chunk_size: usize,

    /// 最大重试次数
    pub max_retries: u8,

    /// 每次重试延迟
    pub retry_delay: Duration,

    /// 保存路径文件夹
    pub state_dir: PathBuf,

    /// 读取文件的缓冲区大小
    pub buffer_size: usize,
}

fn default_state_dir() -> PathBuf {
    dirs::document_dir().unwrap_or_else(|| PathBuf::from("."))
}

impl Default for TusConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            headers: HashMap::new(),
            max_concurrent: 3,
            chunk_size: 1024 * 1024 * 5,
            max_retries: 3,
            retry_delay: Duration::from_secs(1),
            state_dir: default_state_dir(),
            buffer_size: 1024 * 1024,
        }
    }
}

impl TusConfig {
    pub fn new(endpoint: String) -> Self {
        Self {
            endpoint,
            ..Default::default()
        }
    }

    pub fn validate(&self) -> UploadResult<()> {
        // Validate endpoint
        if self.endpoint.is_empty() {
            return Err(UploadError::ConfigError("Endpoint URL cannot be empty".into()));
        }
        if !self.endpoint.starts_with("http://") && !self.endpoint.starts_with("https://") {
            return Err(UploadError::ConfigError("Endpoint URL must start with http:// or https://".into()));
        }

        // Validate concurrent uploads
        if self.max_concurrent == 0 {
            return Err(UploadError::ConfigError("Max concurrent uploads must be greater than 0".into()));
        }

        // Validate chunk size
        if self.chunk_size == 0 {
            return Err(UploadError::ConfigError("Chunk size must be greater than 0".into()));
        }
        if self.chunk_size > 100 * 1024 * 1024 {
            return Err(UploadError::ConfigError("Chunk size cannot be larger than 100MB".into()));
        }

        // Validate buffer size
        if self.buffer_size == 0 {
            return Err(UploadError::ConfigError("Buffer size must be greater than 0".into()));
        }
        if self.buffer_size > self.chunk_size {
            return Err(UploadError::ConfigError("Buffer size cannot be larger than chunk size".into()));
        }

        Ok(())
    }

    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers.extend(headers);
        self
    }
}
